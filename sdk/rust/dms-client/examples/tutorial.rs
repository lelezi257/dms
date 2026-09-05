//! 普通同步应用：只用公开 SDK 接口，串起用户能看到的业务语义。
//! 在 Linux VM 启动 Node/Meta 后设置 DMS_ENDPOINT，再执行本例。

use dms_client::{
    ByteRange, ClientOptions, DmsClient, GetOptions, HashEntry, HashReadVersion, HashScanOptions,
    HashWriteMode, HashWriteOptions, KvEntry, ReadVersion, ScanCursor, SetOptions, WriteCondition,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = DmsClient::connect_with_options(ClientOptions::default())?;
    let original = client.set("tutorial/value", b"abcdefghij")?;
    assert_eq!(
        client.get("tutorial/value")?.as_deref(),
        Some(&b"abcdefghij"[..])
    );

    // 显式条件写：仅当 Current 仍是 original 时才能更新，条件检查由 Meta 完成。
    let updated = client.set_with_options(
        "tutorial/value",
        b"abcdefghij",
        SetOptions {
            condition: WriteCondition::IfVersion(original.version),
            ..Default::default()
        },
    )?;
    assert!(updated.version > original.version);
    client.set_range("tutorial/value", 4, b"X")?;
    assert_eq!(
        client.get("tutorial/value")?.as_deref(),
        Some(&b"abcdXfghij"[..])
    );

    // 显式读取旧版本的一段 bytes；当前实验保留策略尚未淘汰这个版本。
    let old = client
        .get_with_options(
            "tutorial/value",
            GetOptions {
                version: ReadVersion::Exact(original.version),
                range: Some(ByteRange { offset: 3, len: 4 }),
            },
        )?
        .ok_or("historical value missing")?;
    assert_eq!(old.bytes, b"defg");

    // MSET 原子发布多个 key；MGET 保持顺序，但不是跨 key 同一快照。
    client.del("tutorial/missing")?;
    client.mset(
        &[
            KvEntry::new("tutorial/a", b"A".to_vec())?,
            KvEntry::new("tutorial/b", b"B".to_vec())?,
        ],
        Default::default(),
    )?;
    let batch = client.mget(&["tutorial/a", "tutorial/missing", "tutorial/b"])?;
    assert_eq!(
        batch[0].as_ref().map(|v| v.bytes.as_slice()),
        Some(&b"A"[..])
    );
    assert!(batch[1].is_none());
    assert_eq!(
        batch[2].as_ref().map(|v| v.bytes.as_slice()),
        Some(&b"B"[..])
    );

    // 先 Replace 建立确定的字段集，使本例可以重复运行，再演示 Merge 保留其它字段。
    client.hset(
        "tutorial/job",
        &[
            HashEntry::new("model", b"A".to_vec())?,
            HashEntry::new("config", b"B".to_vec())?,
        ],
        HashWriteOptions {
            mode: HashWriteMode::Replace,
            ..Default::default()
        },
    )?;
    client.hset(
        "tutorial/job",
        &[HashEntry::new("model", b"C".to_vec())?],
        Default::default(),
    )?;
    assert_eq!(
        client
            .hget("tutorial/job", "config")?
            .ok_or("config missing")?
            .bytes,
        b"B"
    );

    // 每页最多一个字段；第一页绑定版本，后续继续同一 Exact 版本。
    let mut cursor = ScanCursor(0);
    let mut version = HashReadVersion::Current;
    let mut seen = Vec::new();
    loop {
        let page = client.hscan(
            "tutorial/job",
            cursor,
            HashScanOptions { version, limit: 1 },
        )?;
        version = HashReadVersion::Exact(page.version.ok_or("hash version missing")?);
        seen.extend(
            page.entries
                .into_iter()
                .map(|value| (value.field.as_bytes().to_vec(), value.bytes)),
        );
        cursor = page.next_cursor;
        if cursor.0 == 0 {
            break;
        }
    }
    seen.sort();
    assert_eq!(
        seen,
        vec![
            (b"config".to_vec(), b"B".to_vec()),
            (b"model".to_vec(), b"C".to_vec())
        ]
    );

    client.hset(
        "tutorial/job",
        &[HashEntry::new("model", b"D".to_vec())?],
        HashWriteOptions {
            mode: HashWriteMode::Replace,
            ..Default::default()
        },
    )?;
    assert!(client.hget("tutorial/job", "config")?.is_none());
    assert_eq!(
        client
            .hget("tutorial/job", "model")?
            .ok_or("model missing")?
            .bytes,
        b"D"
    );

    // 删除逻辑 key，不承诺立刻物理回收 Block；连续 SET/GET 无需添加 sleep。
    for key in [
        "tutorial/value",
        "tutorial/a",
        "tutorial/b",
        "tutorial/job",
        "tutorial/missing",
    ] {
        client.del(key)?;
        assert!(client.get(key)?.is_none());
        assert!(!client.del(key)?.deleted);
    }
    println!("DMS tutorial passed");
    Ok(())
}
