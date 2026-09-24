use serde_json::json;
use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

const STATE_VERSION: &str = "DMS_HOME_CENTER_V2";

#[derive(Clone, Debug, Eq, PartialEq)]
struct NodeInfo {
    nfs_endpoint: String,
    p2p_endpoint: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RootStatus {
    Pending,
    Active,
    Deleting,
    Tombstone,
}

impl RootStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Deleting => "deleting",
            Self::Tombstone => "tombstone",
        }
    }

    fn parse(value: &str) -> io::Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            "deleting" => Ok(Self::Deleting),
            "tombstone" => Ok(Self::Tombstone),
            _ => Err(invalid_state("unknown root state")),
        }
    }

    fn query_prefix(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Active => "",
            Self::Deleting => "DELETING",
            Self::Tombstone => "TOMBSTONE",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RootInfo {
    owner: String,
    status: RootStatus,
    generation: u64,
}

#[derive(Clone, Debug, Default)]
struct State {
    nodes: HashMap<String, NodeInfo>,
    roots: HashMap<String, RootInfo>,
}

struct Center {
    state: Mutex<State>,
    state_file: Option<PathBuf>,
    failed: Mutex<Option<String>>,
    token: String,
}

pub fn serve_durable(
    rpc_addr: &str,
    management_http_addr: &str,
    state_file: &Path,
) -> io::Result<()> {
    let token = env::var("DMS_HOME_TOKEN").map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "DMS_HOME_TOKEN is required for center",
        )
    })?;
    if token.len() < 16 || has_whitespace(&token) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "DMS_HOME_TOKEN must be at least 16 bytes without whitespace",
        ));
    }
    let state = load_state(state_file)?;
    let center = Arc::new(Center {
        state: Mutex::new(state),
        state_file: Some(state_file.to_path_buf()),
        failed: Mutex::new(None),
        token,
    });
    // Both advertised interfaces must be available before the center starts serving roots.
    let rpc_listener = TcpListener::bind(rpc_addr)?;
    let management_listener = TcpListener::bind(management_http_addr)?;
    let management = center.clone();
    thread::spawn(move || {
        if let Err(error) = serve_management_http(management_listener, management) {
            eprintln!("center management HTTP stopped: {error}");
        }
    });
    serve_rpc(rpc_listener, center)
}

fn serve_rpc(listener: TcpListener, center: Arc<Center>) -> io::Result<()> {
    eprintln!("home center rpc listening on {}", listener.local_addr()?);
    for incoming in listener.incoming() {
        let stream = incoming?;
        let center = center.clone();
        thread::spawn(move || {
            if let Err(error) = handle_rpc(stream, center) {
                eprintln!("center request: {error}");
            }
        });
    }
    Ok(())
}

fn handle_rpc(mut stream: TcpStream, center: Arc<Center>) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut line = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut line)?;
    let response = center.process_wire(line.trim_end());
    stream.write_all(response.as_bytes())
}

impl Center {
    fn process_wire(&self, request: &str) -> String {
        let (command, authenticated) = match request.split_once(' ') {
            Some(("AUTH", rest)) => match rest.split_once(' ') {
                Some((token, command)) if token == self.token => (command, true),
                _ => return "ERROR AUTH\n".to_owned(),
            },
            _ => (request, false),
        };
        if mutating_command(command) && !authenticated {
            return "ERROR AUTH\n".to_owned();
        }
        self.process(command)
    }

    fn process(&self, request: &str) -> String {
        if let Some(error) = self.failed.lock().unwrap().as_ref() {
            return format!("ERROR CENTER_FAILED {}\n", one_line(error));
        }
        let parts: Vec<_> = request.split_whitespace().collect();
        let result = match parts.as_slice() {
            ["NODE", id, nfs_endpoint, p2p_endpoint] => self.register_node(
                id,
                NodeInfo {
                    nfs_endpoint: (*nfs_endpoint).to_owned(),
                    p2p_endpoint: (*p2p_endpoint).to_owned(),
                },
            ),
            ["NODES"] => self.nodes(),
            ["RESERVE", name, owner] => self.reserve_root(name, owner),
            ["ABORT_PENDING", name, owner] => self.abort_pending(name, owner),
            ["ACTIVATE", name, owner] => self.activate_root(name, owner),
            ["GET", name] => self.get_root(name),
            ["ROOTS"] => self.roots(),
            ["OWNED", owner] => self.owned_roots(owner),
            ["DELETE_PREPARE", name, owner] => self.delete_prepare(name, owner),
            ["DELETE_COMMIT", name, owner] => self.delete_commit(name, owner),
            ["DELETE", name] => self.delete_root(name, None),
            ["DELETE", name, owner] => self.delete_root(name, Some(owner)),
            ["ROOT", name, owner] => self.legacy_root(name, owner),
            ["DEL", name, owner] => self.delete_root(name, Some(owner)),
            _ => Ok("ERROR\n".to_owned()),
        };
        match result {
            Ok(response) => response,
            Err(error) => format!("ERROR {}\n", one_line(&error.to_string())),
        }
    }

    fn register_node(&self, id: &str, info: NodeInfo) -> io::Result<String> {
        if !safe(id) || has_whitespace(&info.nfs_endpoint) || has_whitespace(&info.p2p_endpoint) {
            return Ok("ERROR\n".to_owned());
        }
        let mut state = self.state.lock().unwrap();
        state.nodes.insert(id.to_owned(), info);
        Ok("OK\n".to_owned())
    }

    fn nodes(&self) -> io::Result<String> {
        let state = self.state.lock().unwrap();
        let mut rows: Vec<_> = state
            .nodes
            .iter()
            .map(|(id, info)| format!("{id} {} {}", info.nfs_endpoint, info.p2p_endpoint))
            .collect();
        rows.sort();
        Ok(format!("{}\n", rows.join("\n")))
    }

    fn reserve_root(&self, name: &str, owner: &str) -> io::Result<String> {
        self.change_roots(|state| {
            validate_root_and_owner(state, name, owner)?;
            match state.roots.get(name) {
                Some(root)
                    if root.owner != owner
                        || matches!(root.status, RootStatus::Deleting | RootStatus::Tombstone) =>
                {
                    Ok(format!(
                        "CONFLICT {} {}\n",
                        root.owner,
                        root.status.as_str()
                    ))
                }
                Some(_) => Ok("OK\n".to_owned()),
                None => {
                    let generation = next_generation(state);
                    state.roots.insert(
                        name.to_owned(),
                        RootInfo {
                            owner: owner.to_owned(),
                            status: RootStatus::Pending,
                            generation,
                        },
                    );
                    Ok("OK\n".to_owned())
                }
            }
        })
    }

    fn activate_root(&self, name: &str, owner: &str) -> io::Result<String> {
        self.change_roots(|state| {
            validate_root_and_owner(state, name, owner)?;
            match state.roots.get_mut(name) {
                Some(root)
                    if root.owner == owner
                        && matches!(root.status, RootStatus::Pending | RootStatus::Active) =>
                {
                    root.status = RootStatus::Active;
                    Ok("OK\n".to_owned())
                }
                Some(root) => Ok(format!(
                    "CONFLICT {} {}\n",
                    root.owner,
                    root.status.as_str()
                )),
                None => Ok("MISSING\n".to_owned()),
            }
        })
    }

    fn abort_pending(&self, name: &str, owner: &str) -> io::Result<String> {
        self.change_roots(|state| {
            validate_root_and_owner(state, name, owner)?;
            match state.roots.get(name) {
                Some(root) if root.owner == owner && root.status == RootStatus::Pending => {
                    state.roots.remove(name);
                    Ok("OK\n".to_owned())
                }
                Some(root) => Ok(format!(
                    "CONFLICT {} {}\n",
                    root.owner,
                    root.status.as_str()
                )),
                None => Ok("MISSING\n".to_owned()),
            }
        })
    }

    fn legacy_root(&self, name: &str, owner: &str) -> io::Result<String> {
        self.change_roots(|state| {
            validate_root_and_owner(state, name, owner)?;
            match state.roots.get_mut(name) {
                Some(root)
                    if root.owner == owner
                        && matches!(root.status, RootStatus::Pending | RootStatus::Active) =>
                {
                    root.status = RootStatus::Active;
                    Ok("OK\n".to_owned())
                }
                Some(root) => Ok(format!(
                    "CONFLICT {} {}\n",
                    root.owner,
                    root.status.as_str()
                )),
                None => {
                    let generation = next_generation(state);
                    state.roots.insert(
                        name.to_owned(),
                        RootInfo {
                            owner: owner.to_owned(),
                            status: RootStatus::Active,
                            generation,
                        },
                    );
                    Ok("OK\n".to_owned())
                }
            }
        })
    }

    fn get_root(&self, name: &str) -> io::Result<String> {
        if !safe(name) {
            return Ok("ERROR\n".to_owned());
        }
        let state = self.state.lock().unwrap();
        let response = match state.roots.get(name) {
            Some(root) if root.status == RootStatus::Active => format!("{}\n", root.owner),
            Some(root) => format!("{} {}\n", root.status.query_prefix(), root.owner),
            None => "MISSING\n".to_owned(),
        };
        Ok(response)
    }

    fn roots(&self) -> io::Result<String> {
        let state = self.state.lock().unwrap();
        let mut rows: Vec<_> = state
            .roots
            .iter()
            .filter(|(_, root)| root.status == RootStatus::Active)
            .map(|(name, root)| format!("{name} {}", root.owner))
            .collect();
        rows.sort();
        Ok(format!("{}\n", rows.join("\n")))
    }

    fn owned_roots(&self, owner: &str) -> io::Result<String> {
        if !safe(owner) {
            return Ok("ERROR\n".to_owned());
        }
        let state = self.state.lock().unwrap();
        let mut rows: Vec<_> = state
            .roots
            .iter()
            .filter(|(_, root)| root.owner == owner)
            .map(|(name, root)| format!("{name} {}", root.status.as_str()))
            .collect();
        rows.sort();
        Ok(format!("{}\n", rows.join("\n")))
    }

    fn delete_root(&self, name: &str, owner: Option<&str>) -> io::Result<String> {
        self.change_roots(|state| {
            if !safe(name) || owner.is_some_and(|value| !safe(value)) {
                return Ok("ERROR\n".to_owned());
            }
            match state.roots.get_mut(name) {
                Some(root) if owner.is_none_or(|value| value == root.owner) => {
                    if root.status == RootStatus::Pending {
                        state.roots.remove(name);
                    } else {
                        root.status = RootStatus::Tombstone;
                    }
                    Ok("OK\n".to_owned())
                }
                Some(root) => Ok(format!(
                    "CONFLICT {} {}\n",
                    root.owner,
                    root.status.as_str()
                )),
                None => Ok("MISSING\n".to_owned()),
            }
        })
    }

    fn delete_prepare(&self, name: &str, owner: &str) -> io::Result<String> {
        self.change_roots(|state| {
            validate_root_and_owner(state, name, owner)?;
            match state.roots.get_mut(name) {
                Some(root)
                    if root.owner == owner
                        && matches!(
                            root.status,
                            RootStatus::Pending | RootStatus::Active | RootStatus::Deleting
                        ) =>
                {
                    root.status = RootStatus::Deleting;
                    Ok("OK\n".to_owned())
                }
                Some(root) if root.owner == owner && root.status == RootStatus::Tombstone => {
                    Ok("TOMBSTONE\n".to_owned())
                }
                Some(root) => Ok(format!(
                    "CONFLICT {} {}\n",
                    root.owner,
                    root.status.as_str()
                )),
                None => Ok("MISSING\n".to_owned()),
            }
        })
    }

    fn delete_commit(&self, name: &str, owner: &str) -> io::Result<String> {
        self.change_roots(|state| {
            validate_root_and_owner(state, name, owner)?;
            match state.roots.get_mut(name) {
                Some(root) if root.owner == owner && root.status == RootStatus::Deleting => {
                    root.status = RootStatus::Tombstone;
                    Ok("OK\n".to_owned())
                }
                Some(root) if root.owner == owner && root.status == RootStatus::Tombstone => {
                    Ok("OK\n".to_owned())
                }
                Some(root) if root.owner == owner => Ok(format!(
                    "NOT_PREPARED {} {}\n",
                    root.owner,
                    root.status.as_str()
                )),
                Some(root) => Ok(format!(
                    "CONFLICT {} {}\n",
                    root.owner,
                    root.status.as_str()
                )),
                None => Ok("MISSING\n".to_owned()),
            }
        })
    }

    fn change_roots<F>(&self, update: F) -> io::Result<String>
    where
        F: FnOnce(&mut State) -> io::Result<String>,
    {
        let mut state = self.state.lock().unwrap();
        let before = state.roots.clone();
        let response = update(&mut state)?;
        if state.roots == before {
            return Ok(response);
        }
        if let Err(error) = persist_configured_state(self.state_file.as_deref(), &state) {
            *self.failed.lock().unwrap() = Some(error.to_string());
            return Err(error);
        }
        Ok(response)
    }
}

fn validate_root_and_owner(state: &State, name: &str, owner: &str) -> io::Result<()> {
    if !safe(name) || !safe(owner) {
        return Err(invalid_state("invalid root or owner"));
    }
    if !state.nodes.contains_key(owner) {
        return Err(invalid_state("unknown owner node"));
    }
    Ok(())
}

fn next_generation(state: &State) -> u64 {
    state
        .roots
        .values()
        .map(|root| root.generation)
        .max()
        .unwrap_or(0)
        + 1
}

fn load_state(path: &Path) -> io::Result<State> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(State::default()),
        Err(error) => return Err(error),
    };
    let (payload, root_count) = verify_state_footer(&content)?;
    let mut lines = payload.lines();
    if lines.next() != Some(STATE_VERSION) {
        return Err(invalid_state("bad state header"));
    }
    let mut state = State::default();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<_> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["ROOT", name, owner, status, generation] if safe(name) && safe(owner) => {
                if state.roots.contains_key(*name) {
                    return Err(invalid_state("duplicate root"));
                }
                let generation = generation
                    .parse::<u64>()
                    .map_err(|_| invalid_state("bad generation"))?;
                state.roots.insert(
                    (*name).to_owned(),
                    RootInfo {
                        owner: (*owner).to_owned(),
                        status: RootStatus::parse(status)?,
                        generation,
                    },
                );
            }
            _ => return Err(invalid_state("malformed state row")),
        }
    }
    if state.roots.len() != root_count {
        return Err(invalid_state("root count mismatch"));
    }
    Ok(state)
}

fn persist_state(path: &Path, state: &State) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| invalid_state("state path has no file name"))?;
    let tmp = parent.join(format!(".{file_name}.tmp.{}", std::process::id()));
    {
        let mut file = File::create(&tmp)?;
        file.write_all(render_state(state).as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    sync_directory(parent)
}

fn persist_configured_state(path: Option<&Path>, state: &State) -> io::Result<()> {
    if let Some(path) = path {
        persist_state(path, state)?;
    }
    Ok(())
}

fn render_state(state: &State) -> String {
    let mut rows = vec![format!("{STATE_VERSION}\n")];
    let mut roots: Vec<_> = state.roots.iter().collect();
    roots.sort_by(|left, right| left.0.cmp(right.0));
    for (name, root) in roots {
        rows.push(format!(
            "ROOT {name} {} {} {}\n",
            root.owner,
            root.status.as_str(),
            root.generation
        ));
    }
    let payload = rows.concat();
    let checksum = crc32fast::hash(payload.as_bytes());
    format!("{payload}CRC32 {checksum:08x} {}\n", state.roots.len())
}

fn verify_state_footer(content: &str) -> io::Result<(String, usize)> {
    let without_trailing_newline = content
        .strip_suffix('\n')
        .ok_or_else(|| invalid_state("state missing final newline"))?;
    let (payload_without_final_newline, footer) = without_trailing_newline
        .rsplit_once('\n')
        .ok_or_else(|| invalid_state("state missing footer"))?;
    let payload = format!("{payload_without_final_newline}\n");
    let parts: Vec<_> = footer.split_whitespace().collect();
    let ["CRC32", expected, root_count] = parts.as_slice() else {
        return Err(invalid_state("bad state footer"));
    };
    let expected = u32::from_str_radix(expected, 16).map_err(|_| invalid_state("bad checksum"))?;
    let actual = crc32fast::hash(payload.as_bytes());
    if actual != expected {
        return Err(invalid_state("state checksum mismatch"));
    }
    let root_count = root_count
        .parse::<usize>()
        .map_err(|_| invalid_state("bad root count"))?;
    Ok((payload, root_count))
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn serve_management_http(listener: TcpListener, center: Arc<Center>) -> io::Result<()> {
    eprintln!(
        "home center management listening on {}",
        listener.local_addr()?
    );
    for incoming in listener.incoming() {
        let stream = incoming?;
        let center = center.clone();
        thread::spawn(move || {
            if let Err(error) = handle_management_http(stream, center) {
                eprintln!("center management request: {error}");
            }
        });
    }
    Ok(())
}

fn handle_management_http(mut stream: TcpStream, center: Arc<Center>) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut first_line = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut first_line)?;
    let fields: Vec<_> = first_line.split_whitespace().collect();
    let (status, body) = match fields.as_slice() {
        ["GET", "/v1/roots", _] => (200, management_roots(&center)),
        ["GET", path, _] if path.starts_with("/v1/roots/") => {
            match management_root(&center, &path["/v1/roots/".len()..]) {
                Some(body) => (200, body),
                None => (404, json!({"error": "missing root"})),
            }
        }
        _ => (404, json!({"error": "not found"})),
    };
    write_http_json(&mut stream, status, &body)
}

fn management_roots(center: &Center) -> serde_json::Value {
    let state = center.state.lock().unwrap();
    let mut names: Vec<_> = state.roots.keys().cloned().collect();
    names.sort();
    let roots: Vec<_> = names
        .iter()
        .filter_map(|name| root_json(name, &state))
        .collect();
    json!({ "roots": roots })
}

fn management_root(center: &Center, raw_name: &str) -> Option<serde_json::Value> {
    let name = percent_decode(raw_name).ok()?;
    if !safe(&name) {
        return None;
    }
    let state = center.state.lock().unwrap();
    root_json(&name, &state)
}

fn root_json(name: &str, state: &State) -> Option<serde_json::Value> {
    let root = state.roots.get(name)?;
    let node = state.nodes.get(&root.owner).map(|info| {
        json!({
            "id": root.owner,
            "nfs_endpoint": info.nfs_endpoint,
            "p2p_endpoint": info.p2p_endpoint,
        })
    });
    Some(json!({
        "name": name,
        "owner": root.owner,
        "status": root.status.as_str(),
        "generation": root.generation,
        "node": node,
    }))
}

fn write_http_json(
    stream: &mut TcpStream,
    status: u16,
    body: &serde_json::Value,
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let bytes = serde_json::to_vec(body)?;
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        bytes.len()
    )?;
    stream.write_all(&bytes)
}

fn percent_decode(value: &str) -> io::Result<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let hi = input
                .next()
                .ok_or_else(|| invalid_state("bad percent escape"))?;
            let lo = input
                .next()
                .ok_or_else(|| invalid_state("bad percent escape"))?;
            bytes.push((hex(hi)? << 4) | hex(lo)?);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).map_err(|_| invalid_state("bad utf-8"))
}

fn hex(byte: u8) -> io::Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(invalid_state("bad hex digit")),
    }
}

pub fn query(address: &str, request: &str) -> io::Result<String> {
    let address: SocketAddr = address.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "center endpoint must be an IP:port",
        )
    })?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    if mutating_command(request) {
        let token = env::var("DMS_HOME_TOKEN").map_err(|_| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "DMS_HOME_TOKEN is required for center mutation",
            )
        })?;
        if token.len() < 16 || has_whitespace(&token) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid DMS_HOME_TOKEN",
            ));
        }
        stream.write_all(format!("AUTH {token} {request}\n").as_bytes())?;
    } else {
        stream.write_all(format!("{request}\n").as_bytes())?;
    }
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response.trim_end().to_owned())
}

fn mutating_command(request: &str) -> bool {
    matches!(
        request.split_whitespace().next(),
        Some(
            "NODE"
                | "RESERVE"
                | "ABORT_PENDING"
                | "ACTIVATE"
                | "DELETE_PREPARE"
                | "DELETE_COMMIT"
                | "DELETE"
                | "ROOT"
                | "DEL"
        )
    )
}

pub fn safe(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains(char::is_whitespace)
        && name != "."
        && name != ".."
}

fn has_whitespace(value: &str) -> bool {
    value.is_empty() || value.contains(char::is_whitespace)
}

fn invalid_state(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn one_line(value: &str) -> String {
    value.replace(char::is_whitespace, "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn reject_ambiguous_components() {
        for bad in ["", ".", "..", "a/b", "a b", "a\nb"] {
            assert!(!safe(bad));
        }
        assert!(safe("job-42"));
    }

    #[test]
    fn reserve_activate_and_recover_roots() {
        let dir = test_dir("center-recover");
        let state_file = dir.join("center.state");
        let center = durable_center(&state_file);
        assert_eq!(
            center.process("NODE A nfs://a 127.0.0.1:9001").trim_end(),
            "OK"
        );
        assert_eq!(center.process("RESERVE job-42 A").trim_end(), "OK");
        assert_eq!(center.process("OWNED A").trim_end(), "job-42 pending");
        assert_eq!(center.process("GET job-42").trim_end(), "PENDING A");
        assert_eq!(center.process("ROOTS").trim_end(), "");
        assert_eq!(center.process("ACTIVATE job-42 A").trim_end(), "OK");
        assert_eq!(center.process("OWNED A").trim_end(), "job-42 active");
        assert_eq!(center.process("GET job-42").trim_end(), "A");

        let recovered = load_state(&state_file).unwrap();
        let root = recovered.roots.get("job-42").unwrap();
        assert_eq!(root.owner, "A");
        assert_eq!(root.status, RootStatus::Active);
        cleanup(dir);
    }

    #[test]
    fn concurrent_reserve_keeps_single_owner() {
        let dir = test_dir("center-race");
        let state_file = dir.join("center.state");
        let center = durable_center(&state_file);
        assert_eq!(center.process("NODE A nfs://a p2p-a").trim_end(), "OK");
        assert_eq!(center.process("NODE B nfs://b p2p-b").trim_end(), "OK");
        let shared = Arc::new(center);
        let mut handles = Vec::new();
        for owner in ["A", "B"] {
            let center = shared.clone();
            handles.push(thread::spawn(move || {
                center.process(&format!("RESERVE shared {owner}"))
            }));
        }
        let responses: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            responses
                .iter()
                .filter(|response| response.trim_end() == "OK")
                .count(),
            1
        );
        assert_eq!(
            responses
                .iter()
                .filter(|response| response.starts_with("CONFLICT "))
                .count(),
            1
        );
        cleanup(dir);
    }

    #[test]
    fn corrupt_state_refuses_to_load() {
        let dir = test_dir("center-corrupt");
        let state_file = dir.join("center.state");
        fs::write(&state_file, "not a state file\n").unwrap();
        assert_eq!(
            load_state(&state_file).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        cleanup(dir);
    }

    #[test]
    fn checksum_footer_rejects_silent_root_loss() {
        let dir = test_dir("center-checksum");
        let state_file = dir.join("center.state");
        let center = durable_center(&state_file);
        assert_eq!(center.process("NODE A nfs://a p2p-a").trim_end(), "OK");
        assert_eq!(center.process("ROOT one A").trim_end(), "OK");
        assert_eq!(center.process("ROOT two A").trim_end(), "OK");
        let content = fs::read_to_string(&state_file).unwrap();
        let damaged = content.replace("ROOT two A active 2\n", "");
        fs::write(&state_file, damaged).unwrap();
        assert_eq!(
            load_state(&state_file).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        cleanup(dir);
    }

    #[test]
    fn delete_prepare_hides_active_root_and_survives_restart() {
        let dir = test_dir("center-delete-prepare");
        let state_file = dir.join("center.state");
        let center = durable_center(&state_file);
        assert_eq!(center.process("NODE A nfs://a p2p-a").trim_end(), "OK");
        assert_eq!(center.process("NODE B nfs://b p2p-b").trim_end(), "OK");
        assert_eq!(center.process("ROOT job-42 A").trim_end(), "OK");
        assert_eq!(center.process("DELETE_PREPARE job-42 A").trim_end(), "OK");
        assert_eq!(center.process("GET job-42").trim_end(), "DELETING A");
        assert_eq!(center.process("ROOTS").trim_end(), "");
        assert!(
            center
                .process("RESERVE job-42 B")
                .starts_with("CONFLICT A deleting")
        );

        let recovered = load_state(&state_file).unwrap();
        let root = recovered.roots.get("job-42").unwrap();
        assert_eq!(root.owner, "A");
        assert_eq!(root.status, RootStatus::Deleting);
        cleanup(dir);
    }

    #[test]
    fn delete_commit_tombstones_and_blocks_reuse() {
        let dir = test_dir("center-delete-commit");
        let state_file = dir.join("center.state");
        let center = durable_center(&state_file);
        assert_eq!(center.process("NODE A nfs://a p2p-a").trim_end(), "OK");
        assert_eq!(center.process("NODE B nfs://b p2p-b").trim_end(), "OK");
        assert_eq!(center.process("ROOT job-42 A").trim_end(), "OK");
        assert_eq!(center.process("DELETE_PREPARE job-42 A").trim_end(), "OK");
        assert_eq!(center.process("DELETE_COMMIT job-42 A").trim_end(), "OK");
        assert_eq!(center.process("GET job-42").trim_end(), "TOMBSTONE A");
        assert_eq!(center.process("ROOTS").trim_end(), "");
        assert!(
            center
                .process("RESERVE job-42 A")
                .starts_with("CONFLICT A tombstone")
        );
        assert!(
            center
                .process("RESERVE job-42 B")
                .starts_with("CONFLICT A tombstone")
        );
        assert_eq!(center.process("DELETE_COMMIT job-42 A").trim_end(), "OK");
        cleanup(dir);
    }

    #[test]
    fn legacy_delete_aborts_pending_but_tombstones_active() {
        let dir = test_dir("center-legacy-delete");
        let state_file = dir.join("center.state");
        let center = durable_center(&state_file);
        assert_eq!(center.process("NODE A nfs://a p2p-a").trim_end(), "OK");
        assert_eq!(center.process("RESERVE pending A").trim_end(), "OK");
        assert_eq!(center.process("DELETE pending A").trim_end(), "OK");
        assert_eq!(center.process("GET pending").trim_end(), "MISSING");

        assert_eq!(center.process("ROOT active A").trim_end(), "OK");
        assert_eq!(center.process("DELETE active A").trim_end(), "OK");
        assert_eq!(center.process("GET active").trim_end(), "TOMBSTONE A");
        cleanup(dir);
    }

    #[test]
    fn abort_pending_only_removes_unmaterialized_reservation() {
        let dir = test_dir("center-abort-pending");
        let state_file = dir.join("center.state");
        let center = durable_center(&state_file);
        assert_eq!(center.process("NODE A nfs://a p2p-a").trim_end(), "OK");
        assert_eq!(center.process("NODE B nfs://b p2p-b").trim_end(), "OK");
        assert_eq!(center.process("RESERVE job A").trim_end(), "OK");
        assert_eq!(
            center.process("ABORT_PENDING job B").trim_end(),
            "CONFLICT A pending"
        );
        assert_eq!(center.process("ABORT_PENDING job A").trim_end(), "OK");
        assert_eq!(center.process("GET job").trim_end(), "MISSING");
        assert_eq!(center.process("RESERVE job B").trim_end(), "OK");
        assert_eq!(center.process("ACTIVATE job B").trim_end(), "OK");
        assert_eq!(
            center.process("ABORT_PENDING job B").trim_end(),
            "CONFLICT B active"
        );
        assert_eq!(center.process("GET job").trim_end(), "B");
        cleanup(dir);
    }

    #[test]
    fn persist_error_leaves_center_failed_not_rolled_back() {
        let dir = test_dir("center-fail-stop");
        let bad_parent = dir.join("not-a-directory");
        fs::write(&bad_parent, "file").unwrap();
        let center = Center {
            state: Mutex::new(State::default()),
            state_file: Some(bad_parent.join("state")),
            failed: Mutex::new(None),
            token: "test-center-secret".to_owned(),
        };
        assert_eq!(center.process("NODE A nfs://a p2p-a").trim_end(), "OK");
        assert!(center.process("ROOT job-42 A").starts_with("ERROR "));
        assert!(
            center
                .process("GET job-42")
                .starts_with("ERROR CENTER_FAILED ")
        );
        cleanup(dir);
    }

    #[test]
    fn management_json_reports_owner_and_endpoint() {
        let dir = test_dir("center-json");
        let state_file = dir.join("center.state");
        let center = durable_center(&state_file);
        assert_eq!(center.process("NODE A nfs://a p2p-a").trim_end(), "OK");
        assert_eq!(center.process("RESERVE job-42 A").trim_end(), "OK");
        let value = management_root(&center, "job-42").unwrap();
        assert_eq!(value["name"], "job-42");
        assert_eq!(value["owner"], "A");
        assert_eq!(value["status"], "pending");
        assert_eq!(value["node"]["nfs_endpoint"], "nfs://a");
        cleanup(dir);
    }

    fn durable_center(state_file: &Path) -> Center {
        Center {
            state: Mutex::new(load_state(state_file).unwrap()),
            state_file: Some(state_file.to_path_buf()),
            failed: Mutex::new(None),
            token: "test-center-secret".to_owned(),
        }
    }

    #[test]
    fn wire_mutations_require_center_token() {
        let dir = test_dir("center-auth");
        let center = durable_center(&dir.join("center.state"));
        assert_eq!(
            center.process_wire("NODE A 127.0.0.1:/ 127.0.0.1:9001"),
            "ERROR AUTH\n"
        );
        assert_eq!(
            center.process_wire("AUTH wrong NODE A 127.0.0.1:/ 127.0.0.1:9001"),
            "ERROR AUTH\n"
        );
        assert_eq!(
            center.process_wire("AUTH test-center-secret NODE A 127.0.0.1:/ 127.0.0.1:9001"),
            "OK\n"
        );
        assert_eq!(center.process_wire("RESERVE job A"), "ERROR AUTH\n");
        assert_eq!(
            center.process_wire("AUTH test-center-secret RESERVE job A"),
            "OK\n"
        );
        assert_eq!(center.process_wire("GET job"), "PENDING A\n");
        cleanup(dir);
    }

    fn test_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        dir.push(format!("dms-home-{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(path: PathBuf) {
        let _ = fs::remove_dir_all(path);
    }
}
