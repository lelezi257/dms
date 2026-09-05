//! Small runnable application using only the public `dms-client` API.

use dms_client::{ClientOptions, DmsClient, KvEntry};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = std::env::var("DMS_ENDPOINT")
        .unwrap_or_else(|_| "unix:///tmp/dms-local-dev/run/dms-worker.sock".to_string());
    let writer = DmsClient::connect(&endpoint, ClientOptions::default())?;
    let reader = DmsClient::connect(&endpoint, ClientOptions::default())?;
    let key = "checkpoint/latest";

    // 第一轮：writer 发布 v1，reader 从 Node 读取并缓存 v1。
    writer.set(key, b"manifest-v1")?;
    let value = reader.get(key)?;
    if value.as_deref() != Some(b"manifest-v1") {
        return Err("initial SET/GET value mismatch".into());
    }

    // 第二轮：writer 发布 v2。Meta 产生 InvalidateCurrent，Node 经 Session stream
    // 主动通知 reader。同步 SET 在所有本 Node Session ACK 失效事件后才返回，
    // 因此下一行 GET 无需 sleep、轮询或猜测传播时间。
    writer.set(key, b"manifest-v2")?;
    let value = reader.get(key)?;
    if value.as_deref() != Some(b"manifest-v2") {
        return Err("cache invalidation did not expose v2".into());
    }

    let deleted = writer.del(key)?;
    if !deleted.deleted {
        return Err("first DEL must delete the visible value".into());
    }
    let value = reader.get(key)?;
    if value.is_some() {
        return Err("GET after DEL must return None".into());
    }
    let repeated = writer.del(key)?;
    if repeated.deleted {
        return Err("repeated DEL must be an idempotent no-op".into());
    }

    // 超过 inline 阈值，验证 AllocateStaging → TransferEngine → Commit 的
    // 跨进程慢路径，而不是只证明小对象 unary RPC。
    let large_key = "checkpoint/large";
    let large_value = vec![0x5a; 80 * 1024];
    writer.set(large_key, &large_value)?;
    if reader.get(large_key)?.as_deref() != Some(large_value.as_slice()) {
        return Err("staged large-value SET/GET mismatch".into());
    }
    writer.del(large_key)?;

    // Range write：用户仍操作一个 key/value；Node 把 patch 写成新 Block，
    // 以 Extent 覆盖修改区间并按 base version 做 CAS，不复制完整的旧 value。
    let range_key = "checkpoint/range";
    writer.set(range_key, b"0123456789")?;
    writer.set_range(range_key, 3, b"ABC")?;
    if reader.get(range_key)?.as_deref() != Some(b"012ABC6789") {
        return Err("SET_RANGE patched bytes mismatch".into());
    }

    // Multi-key batch：MSET 在 Meta 原子发布全部 key；MGET 保持输入顺序，
    // 但逐 key 解析，所以读结果不承诺跨 key 同一快照。
    writer.mset(
        &[
            KvEntry::new("batch/a", b"A".to_vec())?,
            KvEntry::new("batch/b", b"B".to_vec())?,
        ],
        Default::default(),
    )?;
    let batch = reader.mget(&["batch/a", "batch/b"])?;
    if batch.len() != 2
        || batch[0].as_ref().map(|value| value.bytes.as_slice()) != Some(&b"A"[..])
        || batch[1].as_ref().map(|value| value.bytes.as_slice()) != Some(&b"B"[..])
    {
        return Err("MSET/MGET ordered batch mismatch".into());
    }

    println!("dms-client set/get/del + range + batch APIs passed endpoint={endpoint}");
    Ok(())
}
