//! Reliability fault-controller probe.
//!
//! 这个二进制是可靠性实验的私有工具，不属于 SDK 发布面。它通过 JSONL
//! stdin/stdout 暴露一组稳定的机器命令，让故障控制器能在同一个进程内复用
//! `DmsClient`、连接、session 和 mmap 状态。普通命令只走公开 Rust SDK；
//! `set_fixed_operation` 是 R2 专用穿刺，直接调用私有 WorkerService wire
//! 合同以固定 OperationId，避免为了测试把幂等接口扩散到用户 SDK。

use std::{
    collections::HashMap,
    fs,
    io::{self, BufRead, Read, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use dms_client::{
    ClientOptions, DmsClient, DmsValueReader, GetOptions, ObjectInfo, ScanOptions, SetResult,
    SharedWriteBuffer,
};
use dms_error::{DmsError, ErrorKind};
use dms_protocol::v1 as pb;
use pb::worker_service_client::WorkerServiceClient;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;
use tonic::transport::{Channel, Endpoint};

fn main() {
    let mut probe = match ReliabilityProbe::new() {
        Ok(probe) => probe,
        Err(error) => {
            let _ = writeln!(
                io::stdout(),
                "{}",
                json_response(None, None, Err(error), Value::Null)
            );
            return;
        }
    };

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                println!(
                    "{}",
                    json_response(
                        None,
                        None,
                        Err(DmsError::client_invalid_argument(format!(
                            "failed to read JSONL command: {error}"
                        ))),
                        Value::Null,
                    )
                );
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let command: Result<ProbeCommand, _> = serde_json::from_str(&line);
        let response = match command {
            Ok(command) => probe.handle(command),
            Err(error) => json_response(
                None,
                None,
                Err(DmsError::client_invalid_argument(format!(
                    "invalid JSONL command: {error}"
                ))),
                Value::Null,
            ),
        };
        println!("{response}");
        // JSONL 控制器通常逐行等待响应；显式 flush 避免 stdout 缓冲造成假超时。
        let _ = io::stdout().flush();
    }
}

struct ReliabilityProbe {
    client: Option<DmsClient>,
    endpoint: Option<String>,
    runtime: Runtime,
    direct: Option<DirectWorker>,
    readers: HashMap<String, HeldReader>,
    writes: HashMap<String, SharedWriteBuffer>,
    connect_generation: u64,
}

struct DirectWorker {
    worker: WorkerServiceClient<Channel>,
    session_id: u64,
    initial_view_epoch: u64,
    lease_generation: u64,
}

struct HeldReader {
    body: DmsValueReader,
    version: u64,
    len: u64,
    bytes_read: u64,
}

impl ReliabilityProbe {
    fn new() -> Result<Self, DmsError> {
        Ok(Self {
            client: None,
            endpoint: None,
            runtime: Runtime::new().map_err(|error| {
                DmsError::client_connection_unavailable(format!(
                    "failed to create probe Tokio runtime: {error}"
                ))
            })?,
            direct: None,
            readers: HashMap::new(),
            writes: HashMap::new(),
            connect_generation: 0,
        })
    }

    fn handle(&mut self, command: ProbeCommand) -> Value {
        let run_id = command.run_id.clone();
        let request_id = command.request_id.clone();
        let outcome = match command.op.as_str() {
            "connect" => self.connect(&command),
            "set" => self.set(&command),
            "get" => self.get(&command),
            "stat" => self.stat(&command),
            "scan" => self.scan(&command),
            "del" => self.del(&command),
            "open_reader" => self.open_reader(&command),
            "read_reader" => self.read_reader(&command),
            "close_reader" => self.close_reader(&command),
            "allocate_write" => self.allocate_write(&command),
            "write_shared" => self.write_shared(&command),
            "commit_shared" => self.commit_shared(&command),
            "set_fixed_operation" => self.set_fixed_operation(&command),
            "help" => Ok(help()),
            other => Err(DmsError::client_invalid_argument(format!(
                "unknown probe operation: {other}"
            ))),
        };
        match outcome {
            Ok(fields) => json_response(run_id.as_deref(), request_id.as_deref(), Ok(()), fields),
            Err(error) => json_response(
                run_id.as_deref(),
                request_id.as_deref(),
                Err(error),
                Value::Null,
            ),
        }
    }

    fn connect(&mut self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let endpoint = required_string(command.endpoint.as_deref(), "endpoint")?.to_string();
        self.connect_generation = self.connect_generation.saturating_add(1);
        let options = ClientOptions {
            shared_memory: Some(command.shared_memory.unwrap_or(true)),
            ..ClientOptions::default()
        };
        // R2 固定 OperationId 穿刺只需要 HTTP/TCP unary。UDS/SHM 普通命令仍由
        // 公开 SDK 覆盖；如果后续需要 UDS direct RPC，再在这里补同一私有边界。
        let direct = if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            // R7 会在同一 probe 进程中保留旧 reader，再重启 Node 并建立新的
            // SDK 实例。当前 Node session id 在重启后从小值重新递增；旧实例的
            // 后台 heartbeat 可能把相同 session 的 read_request 水位上报给新
            // Node。这里用私有 direct RPC 预留一小段 session，使新的公开 SDK
            // session 避开旧代次，达到“同一故障控制器，新的 SDK 进程”的隔离效果。
            let reserved_sessions = self.connect_generation.saturating_mul(8);
            Some(
                self.runtime
                    .block_on(connect_direct_worker(&endpoint, reserved_sessions))?,
            )
        } else {
            None
        };
        let client = DmsClient::connect(&endpoint, options).map_err(|error| error.0)?;

        self.client = Some(client);
        self.endpoint = Some(endpoint.clone());
        self.direct = direct;
        // connect 代表新的被测 SDK 进程生命周期。旧 handle 继续绑定上一个
        // Client 会让后续故障证据混淆，所以重连时显式关闭 probe 私有句柄。
        self.readers.clear();
        self.writes.clear();

        Ok(json!({
            "endpoint": endpoint,
            "direct_fixed_operation": self.direct.is_some(),
            "direct_session": self.direct.as_ref().map(|direct| json!({
                "session_id": direct.session_id,
                "initial_view_epoch": direct.initial_view_epoch,
                "lease_generation": direct.lease_generation,
            })),
        }))
    }

    fn set(&self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let key = required_bytes(command, "key")?;
        let value = payload(command)?;
        let result = self.client()?.set(&key, &value)?;
        Ok(set_fields(result, &value))
    }

    fn get(&self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let key = required_bytes(command, "key")?;
        let result = self
            .client()?
            .get_with_options(&key, GetOptions::default())?;
        let Some(result) = result else {
            return Ok(json!({"found": false}));
        };
        if let Some(path) = command.output_file.as_deref() {
            fs::write(path, &result.bytes).map_err(|error| {
                DmsError::client_invalid_argument(format!(
                    "failed to write output_file {}: {error}",
                    path
                ))
            })?;
        }
        Ok(json!({
            "found": true,
            "version": result.version.0,
            "length": result.bytes.len(),
            "digest": digest(&result.bytes),
            "value_hex": optional_value_hex(command, &result.bytes),
        }))
    }

    fn stat(&self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let key = required_bytes(command, "key")?;
        let Some(info) = self.client()?.stat(&key)? else {
            return Ok(json!({"found": false}));
        };
        Ok(json!({
            "found": true,
            "object": object_info_json(&info),
        }))
    }

    fn scan(&self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let prefix = optional_bytes(command, "prefix")?.unwrap_or_default();
        let options = ScanOptions {
            limit: command.limit.unwrap_or(128),
            start_after: command
                .start_after
                .as_deref()
                .map(|_| optional_bytes(command, "start_after"))
                .transpose()?
                .flatten(),
            cursor: command.cursor.clone(),
            delimiter: command
                .delimiter
                .as_deref()
                .map(|_| optional_bytes(command, "delimiter"))
                .transpose()?
                .flatten()
                .unwrap_or_default(),
        };
        let result = self.client()?.scan(&prefix, options)?;
        Ok(json!({
            "items": result.items.iter().map(object_info_json).collect::<Vec<_>>(),
            "next_cursor": result.next_cursor,
        }))
    }

    fn del(&self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let key = required_bytes(command, "key")?;
        let result = self.client()?.del(&key)?;
        Ok(json!({
            "deleted": result.deleted,
            "version": result.version.0,
        }))
    }

    fn open_reader(&mut self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let handle = required_string(command.handle.as_deref(), "handle")?.to_string();
        if self.readers.contains_key(&handle) {
            return Err(DmsError::client_invalid_argument(format!(
                "reader handle already exists: {handle}"
            )));
        }
        let key = required_bytes(command, "key")?;
        let Some(result) = self.client()?.get_reader(&key)? else {
            return Ok(json!({"found": false}));
        };
        let version = result.version.0;
        let len = result.len;
        self.readers.insert(
            handle.clone(),
            HeldReader {
                body: result.body,
                version,
                len,
                bytes_read: 0,
            },
        );
        Ok(json!({
            "found": true,
            "handle": handle,
            "version": version,
            "length": len,
            "bytes_read": 0,
        }))
    }

    fn read_reader(&mut self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let handle = required_string(command.handle.as_deref(), "handle")?.to_string();
        let length = usize::try_from(command.length.unwrap_or(64 * 1024)).map_err(|_| {
            DmsError::client_invalid_argument("read_reader length does not fit usize")
        })?;
        let mut reader = self.readers.remove(&handle).ok_or_else(|| {
            DmsError::client_invalid_argument(format!("unknown reader handle: {handle}"))
        })?;
        let mut buffer = vec![0_u8; length];
        let read = match reader.body.read(&mut buffer) {
            Ok(read) => read,
            Err(error) => {
                // 旧 reader 是 R7 的核心故障证据：Node 重启后它持有的下载票据、
                // read scope 或 mmap 可能已经失效。这里返回原始 numeric DMS
                // 错误，但不再把这个 reader 放回表里，避免旧资源继续影响后续
                // 普通 get/set 的生命周期验证。
                return Err(dms_error_from_io(error));
            }
        };
        buffer.truncate(read);
        reader.bytes_read = reader
            .bytes_read
            .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if let Some(path) = command.output_file.as_deref() {
            fs::write(path, &buffer).map_err(|error| {
                DmsError::client_invalid_argument(format!(
                    "failed to write output_file {}: {error}",
                    path
                ))
            })?;
        }
        let response = json!({
            "handle": handle,
            "version": reader.version,
            "length": reader.len,
            "read": read,
            "bytes_read": reader.bytes_read,
            "eof": read == 0,
            "digest": digest(&buffer),
            "value_hex": optional_value_hex(command, &buffer),
        });
        if read != 0 {
            self.readers.insert(handle, reader);
        }
        Ok(response)
    }

    fn close_reader(&mut self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let handle = required_string(command.handle.as_deref(), "handle")?;
        let reader = self.readers.remove(handle).ok_or_else(|| {
            DmsError::client_invalid_argument(format!("unknown reader handle: {handle}"))
        })?;
        Ok(json!({
            "handle": handle,
            "version": reader.version,
            "length": reader.len,
            "bytes_read": reader.bytes_read,
            "closed": true,
        }))
    }

    fn allocate_write(&mut self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let handle = required_string(command.handle.as_deref(), "handle")?.to_string();
        if self.writes.contains_key(&handle) {
            return Err(DmsError::client_invalid_argument(format!(
                "write handle already exists: {handle}"
            )));
        }
        let key = required_bytes(command, "key")?;
        let len = command
            .length
            .ok_or_else(|| DmsError::client_invalid_argument("allocate_write requires length"))?;
        let len = usize::try_from(len).map_err(|_| {
            DmsError::client_invalid_argument("allocate_write length does not fit usize")
        })?;
        let buffer = self.client()?.allocate_write(&key, len)?;
        let actual_len = buffer.len()?;
        self.writes.insert(handle.clone(), buffer);
        Ok(json!({
            "handle": handle,
            "length": actual_len,
        }))
    }

    fn write_shared(&mut self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let handle = required_string(command.handle.as_deref(), "handle")?;
        let offset = usize::try_from(command.offset.unwrap_or(0)).map_err(|_| {
            DmsError::client_invalid_argument("write_shared offset does not fit usize")
        })?;
        let value = payload(command)?;
        let buffer = self.writes.get_mut(handle).ok_or_else(|| {
            DmsError::client_invalid_argument(format!("unknown write handle: {handle}"))
        })?;
        let slot = buffer.as_mut_slice()?;
        let end = offset.checked_add(value.len()).ok_or_else(|| {
            DmsError::client_invalid_argument("write_shared offset + value length overflowed")
        })?;
        if end > slot.len() {
            return Err(DmsError::client_invalid_argument(format!(
                "write_shared range {}..{} exceeds buffer length {}",
                offset,
                end,
                slot.len()
            )));
        }
        slot[offset..end].copy_from_slice(&value);
        Ok(json!({
            "handle": handle,
            "offset": offset,
            "written": value.len(),
            "digest": digest(&value),
        }))
    }

    fn commit_shared(&mut self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let handle = required_string(command.handle.as_deref(), "handle")?;
        let buffer = self.writes.remove(handle).ok_or_else(|| {
            DmsError::client_invalid_argument(format!("unknown write handle: {handle}"))
        })?;
        let result = self.client()?.commit_shared(buffer)?;
        Ok(json!({
            "handle": handle,
            "version": result.version.0,
            "length": result.len,
        }))
    }

    fn set_fixed_operation(&mut self, command: &ProbeCommand) -> Result<Value, DmsError> {
        let key = required_bytes(command, "key")?;
        let value = payload(command)?;
        let client_instance_id = fixed_client_id(command)?;
        let sequence = command.sequence.ok_or_else(|| {
            DmsError::client_invalid_argument("set_fixed_operation requires sequence")
        })?;
        let node_id = command.node_id.as_deref().ok_or_else(|| {
            DmsError::client_invalid_argument(
                "set_fixed_operation requires node_id to derive private block_id",
            )
        })?;
        let operation_id = operation_identity(&client_instance_id, sequence);
        let block_id = block_identity(node_id, &operation_id);
        let direct = self.direct.as_mut().ok_or_else(|| {
            DmsError::node_transfer_unsupported(
                "set_fixed_operation currently requires an http(s) endpoint from connect",
            )
        })?;
        let response = self
            .runtime
            .block_on(direct.worker.set_inline(pb::SetInlineRequest {
                session_id: direct.session_id,
                key: Some(pb::Key { value: key }),
                value: value.clone(),
                operation_id: Some(pb::OperationId {
                    client_instance_id: client_instance_id.to_vec(),
                    sequence,
                }),
                condition: "any".to_string(),
                durability: "local-memory".to_string(),
            }))
            .map_err(dms_transport::status_to_dms_error)?
            .into_inner();
        Ok(json!({
            "version": response.version,
            "length": response.length,
            "digest": digest(&value),
            "operation": {
                "client_instance_id_hex": encode_hex(&client_instance_id),
                "sequence": sequence,
                "operation_id_hex": encode_hex(&operation_id),
            },
            "block_identity": {
                "node_id": node_id,
                "block_id_hex": encode_hex(&block_id),
            },
            "direct_session": {
                "session_id": direct.session_id,
                "initial_view_epoch": direct.initial_view_epoch,
                "lease_generation": direct.lease_generation,
            },
        }))
    }

    fn client(&self) -> Result<&DmsClient, DmsError> {
        self.client
            .as_ref()
            .ok_or_else(|| DmsError::client_connection_unavailable("probe is not connected"))
    }
}

#[derive(Debug, Deserialize)]
struct ProbeCommand {
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    request_id: Option<String>,
    op: String,

    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    shared_memory: Option<bool>,

    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    key_hex: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    value_hex: Option<String>,
    #[serde(default)]
    value_file: Option<String>,
    #[serde(default)]
    output_file: Option<String>,
    #[serde(default)]
    include_value: Option<bool>,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    length: Option<u64>,
    #[serde(default)]
    offset: Option<u64>,

    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    prefix_hex: Option<String>,
    #[serde(default)]
    start_after: Option<String>,
    #[serde(default)]
    start_after_hex: Option<String>,
    #[serde(default)]
    delimiter: Option<String>,
    #[serde(default)]
    delimiter_hex: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<u32>,

    #[serde(default)]
    client_instance_id_hex: Option<String>,
    #[serde(default)]
    sequence: Option<u64>,
    #[serde(default)]
    node_id: Option<String>,
}

async fn connect_direct_worker(
    endpoint: &str,
    reserved_sessions: u64,
) -> Result<DirectWorker, DmsError> {
    let channel = Endpoint::from_shared(endpoint.to_string())
        .map_err(|error| {
            DmsError::client_invalid_argument(format!(
                "invalid direct endpoint {endpoint}: {error}"
            ))
        })?
        .connect()
        .await
        .map_err(|error| {
            DmsError::client_connection_unavailable(format!(
                "failed to connect direct WorkerService {endpoint}: {error}"
            ))
        })?;
    let mut worker = WorkerServiceClient::new(channel);
    let mut session = None;
    for _ in 0..reserved_sessions.max(1) {
        session = Some(
            worker
                .open_session(pb::OpenSessionRequest {
                    min_version: 1,
                    max_version: 1,
                    shared_memory: false,
                    zero_copy_read: false,
                    zero_copy_write: false,
                    supports_write_lease_release: true,
                })
                .await
                .map_err(dms_transport::status_to_dms_error)?
                .into_inner(),
        );
    }
    let session = session.ok_or_else(|| {
        DmsError::client_connection_unavailable("failed to reserve direct WorkerService session")
    })?;
    Ok(DirectWorker {
        worker,
        session_id: session.session_id,
        initial_view_epoch: session.initial_view_epoch,
        lease_generation: session.lease_generation,
    })
}

fn json_response(
    run_id: Option<&str>,
    request_id: Option<&str>,
    status: Result<(), DmsError>,
    fields: Value,
) -> Value {
    match status {
        Ok(()) => json!({
            "ok": true,
            "run_id": run_id,
            "request_id": request_id,
            "error": Value::Null,
            "result": fields,
        }),
        Err(error) => json!({
            "ok": false,
            "run_id": run_id,
            "request_id": request_id,
            "error": {
                "code": error.code().raw(),
                "code_hex": format!("0x{:08x}", error.code().raw()),
                "kind": error_kind_number(error.kind()),
                "kind_name": format!("{:?}", error.kind()),
                "message": error.message(),
            },
            "result": fields,
        }),
    }
}

fn help() -> Value {
    json!({
        "commands": [
            "connect",
            "set",
            "get",
            "stat",
            "scan",
            "del",
            "open_reader",
            "read_reader",
            "close_reader",
            "allocate_write",
            "write_shared",
            "commit_shared",
            "set_fixed_operation"
        ],
        "payload": "value, value_hex, or value_file",
        "binary_keys": "key or key_hex; scan also supports prefix_hex/start_after_hex/delimiter_hex",
    })
}

fn set_fields(result: SetResult, value: &[u8]) -> Value {
    json!({
        "version": result.version.0,
        "length": result.len,
        "digest": digest(value),
    })
}

fn object_info_json(info: &ObjectInfo) -> Value {
    json!({
        "key_hex": encode_hex(&info.key),
        "key_text": String::from_utf8_lossy(&info.key),
        "length": info.length,
        "version": info.version.0,
        "modified_time_unix_millis": system_time_millis(info.modified_time),
        "is_prefix": info.is_prefix,
    })
}

fn system_time_millis(time: SystemTime) -> i128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as i128,
        Err(error) => -(error.duration().as_millis() as i128),
    }
}

fn required_string<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, DmsError> {
    value.ok_or_else(|| DmsError::client_invalid_argument(format!("{field} is required")))
}

fn required_bytes(command: &ProbeCommand, field: &str) -> Result<Vec<u8>, DmsError> {
    optional_bytes(command, field)?
        .ok_or_else(|| DmsError::client_invalid_argument(format!("{field} is required")))
}

fn optional_bytes(command: &ProbeCommand, field: &str) -> Result<Option<Vec<u8>>, DmsError> {
    let (plain, hex) = match field {
        "key" => (command.key.as_deref(), command.key_hex.as_deref()),
        "prefix" => (command.prefix.as_deref(), command.prefix_hex.as_deref()),
        "start_after" => (
            command.start_after.as_deref(),
            command.start_after_hex.as_deref(),
        ),
        "delimiter" => (
            command.delimiter.as_deref(),
            command.delimiter_hex.as_deref(),
        ),
        other => {
            return Err(DmsError::client_protocol_violation(format!(
                "unknown bytes field selector {other}"
            )));
        }
    };
    if plain.is_some() && hex.is_some() {
        return Err(DmsError::client_invalid_argument(format!(
            "{field} and {field}_hex cannot be set together"
        )));
    }
    if let Some(hex) = hex {
        return decode_hex(hex).map(Some);
    }
    Ok(plain.map(|value| value.as_bytes().to_vec()))
}

fn payload(command: &ProbeCommand) -> Result<Vec<u8>, DmsError> {
    let mut sources = 0;
    sources += usize::from(command.value.is_some());
    sources += usize::from(command.value_hex.is_some());
    sources += usize::from(command.value_file.is_some());
    if sources != 1 {
        return Err(DmsError::client_invalid_argument(
            "exactly one of value, value_hex, or value_file is required",
        ));
    }
    if let Some(value) = command.value.as_deref() {
        return Ok(value.as_bytes().to_vec());
    }
    if let Some(value) = command.value_hex.as_deref() {
        return decode_hex(value);
    }
    let path = command.value_file.as_deref().expect("checked above");
    fs::read(Path::new(path)).map_err(|error| {
        DmsError::client_invalid_argument(format!("failed to read value_file {path}: {error}"))
    })
}

fn dms_error_from_io(error: io::Error) -> DmsError {
    if let Some(error) = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<DmsError>())
    {
        return error.clone();
    }
    DmsError::client_connection_unavailable(format!("reader I/O failed: {error}"))
}

fn fixed_client_id(command: &ProbeCommand) -> Result<[u8; 16], DmsError> {
    let id = required_string(
        command.client_instance_id_hex.as_deref(),
        "client_instance_id_hex",
    )?;
    let bytes = decode_hex(id)?;
    <[u8; 16]>::try_from(bytes.as_slice()).map_err(|_| {
        DmsError::client_invalid_argument("client_instance_id_hex must encode exactly 16 bytes")
    })
}

fn operation_identity(client_instance_id: &[u8; 16], sequence: u64) -> Vec<u8> {
    let mut operation_id = Vec::with_capacity(24);
    operation_id.extend_from_slice(client_instance_id);
    operation_id.extend_from_slice(&sequence.to_be_bytes());
    operation_id
}

fn block_identity(node_id: &str, operation_id: &[u8]) -> Vec<u8> {
    // 与 Node 内部 `block_identity(node_id, operation_id)` 保持同一输入拼接和
    // 稳定摘要算法。这个 helper 只服务可靠性控制器配置 peer-pull fault，
    // 不把 block_id 派生规则扩散到公开 SDK。
    let mut input = Vec::with_capacity(node_id.len() + operation_id.len());
    input.extend_from_slice(node_id.as_bytes());
    input.extend_from_slice(operation_id);
    dms_transport::checksum::stable_digest_bytes(&input).to_vec()
}

fn optional_value_hex(command: &ProbeCommand, bytes: &[u8]) -> Value {
    if command.include_value.unwrap_or(false) {
        json!(encode_hex(bytes))
    } else {
        Value::Null
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("crc32:{:08x}", crc32fast::hash(bytes))
}

fn encode_hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex(input: &str) -> Result<Vec<u8>, DmsError> {
    let clean = input.strip_prefix("0x").unwrap_or(input);
    if !clean.len().is_multiple_of(2) {
        return Err(DmsError::client_invalid_argument(
            "hex input must contain an even number of digits",
        ));
    }
    clean
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_value(pair[0])?;
            let low = hex_value(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_value(byte: u8) -> Result<u8, DmsError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(DmsError::client_invalid_argument(format!(
            "invalid hex digit: {}",
            byte as char
        ))),
    }
}

fn error_kind_number(kind: ErrorKind) -> u32 {
    match kind {
        ErrorKind::Unknown => 0,
        ErrorKind::InvalidArgument => 1,
        ErrorKind::NotFound => 2,
        ErrorKind::AlreadyExists => 3,
        ErrorKind::PermissionDenied => 4,
        ErrorKind::ResourceExhausted => 5,
        ErrorKind::FailedPrecondition => 6,
        ErrorKind::Aborted => 7,
        ErrorKind::Unimplemented => 8,
        ErrorKind::Internal => 9,
        ErrorKind::Unavailable => 10,
        ErrorKind::DataLoss => 11,
        ErrorKind::Unauthenticated => 12,
        ErrorKind::DeadlineExceeded => 13,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip_keeps_binary_payload() {
        let input = b"\x00abc\xff";
        assert_eq!(decode_hex(&encode_hex(input)).unwrap(), input);
    }

    #[test]
    fn command_parser_accepts_minimal_set() {
        let command: ProbeCommand =
            serde_json::from_str(r#"{"request_id":"r1","op":"set","key":"k","value":"v"}"#)
                .unwrap();
        assert_eq!(command.request_id.as_deref(), Some("r1"));
        assert_eq!(required_bytes(&command, "key").unwrap(), b"k");
        assert_eq!(payload(&command).unwrap(), b"v");
    }

    #[test]
    fn command_parser_accepts_reader_handle() {
        let command: ProbeCommand = serde_json::from_str(
            r#"{"request_id":"r1","op":"read_reader","handle":"reader-1","length":4}"#,
        )
        .unwrap();
        assert_eq!(command.handle.as_deref(), Some("reader-1"));
        assert_eq!(command.length, Some(4));
    }

    #[test]
    fn command_parser_accepts_shared_write_handle() {
        let command: ProbeCommand = serde_json::from_str(
            r#"{"request_id":"r1","op":"write_shared","handle":"write-1","offset":2,"value_hex":"ff"}"#,
        )
        .unwrap();
        assert_eq!(command.handle.as_deref(), Some("write-1"));
        assert_eq!(command.offset, Some(2));
        assert_eq!(payload(&command).unwrap(), b"\xff");
    }

    #[test]
    fn command_parser_accepts_fixed_operation_node_id() {
        let command: ProbeCommand = serde_json::from_str(
            r#"{"request_id":"r1","op":"set_fixed_operation","node_id":"node-a","client_instance_id_hex":"000102030405060708090a0b0c0d0e0f","sequence":7,"key":"k","value":"v"}"#,
        )
        .unwrap();
        assert_eq!(command.node_id.as_deref(), Some("node-a"));
        assert_eq!(command.sequence, Some(7));
    }

    #[test]
    fn block_identity_is_stable_and_node_scoped() {
        let client_id = *b"0123456789abcdef";
        let operation = operation_identity(&client_id, 42);
        let first = block_identity("node-a", &operation);
        let second = block_identity("node-a", &operation);
        let other_node = block_identity("node-b", &operation);

        assert_eq!(operation.len(), 24);
        assert_eq!(first.len(), 8);
        assert_eq!(first, second);
        assert_ne!(first, other_node);
    }

    #[test]
    fn response_error_keeps_numeric_code_and_kind() {
        let response = json_response(
            Some("run"),
            Some("req"),
            Err(DmsError::client_invalid_argument("bad input")),
            Value::Null,
        );
        assert_eq!(response["ok"], false);
        assert_eq!(response["run_id"], "run");
        assert_eq!(response["request_id"], "req");
        assert_eq!(response["error"]["kind"], 1);
        assert_eq!(response["error"]["code"].as_u64().unwrap(), 0x0101_0001);
    }

    #[test]
    fn help_mentions_stable_commands() {
        let example = json!({"request_id":"r1","op":"set","key":"k","value":"v"});
        assert!(serde_json::to_string(&example).unwrap().contains("set"));
        let help = help();
        assert!(help["commands"].as_array().unwrap().contains(&json!("get")));
    }
}
