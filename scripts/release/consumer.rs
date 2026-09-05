//! 独立候选包消费者：只依赖 dms-client，不能引用源码树的内部 crate。
use dms_client::{
    ClientOptions, DmsClient, DmsError, ErrorKind, HashEntry, HashWriteMode, HashWriteOptions,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 公共错误类型也必须从唯一 SDK 包导入，无需应用单独依赖内部 common。
    let error = DmsError::client_invalid_argument("candidate package smoke");
    assert_eq!(error.kind(), ErrorKind::InvalidArgument);
    let endpoint = std::env::var("DMS_ENDPOINT")?;
    let shm = std::env::var("DMS_SHARED_MEMORY").as_deref() == Ok("true");
    let client = DmsClient::connect(
        &endpoint,
        ClientOptions {
            shared_memory: Some(shm),
            ..Default::default()
        },
    )?;
    // 同一公开接口验证小值与超过 inline 阈值的大值；写成功后立刻读。
    for size in [10_usize, 128 * 1024] {
        let key = format!("candidate/{shm}/{size}");
        let value: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        client.set(&key, &value)?;
        assert_eq!(client.get(&key)?, Some(value.clone()));
        client.set_range(&key, 4, b"X")?;
        let mut changed = value;
        changed[4] = b'X';
        assert_eq!(client.get(&key)?, Some(changed));
        assert!(client.del(&key)?.deleted);
        assert!(client.get(&key)?.is_none());
        println!("PASS endpoint={endpoint} shm={shm} bytes={size} set/get/set_range/del");
    }
    // 原独立消费者的 Hash 公开类型/HSET 编译检查，在这里同时获得真实读写验证。
    let key = format!("candidate/{shm}/hash");
    client.hset(
        &key,
        &[HashEntry::new("model-0001", b"object-location")?],
        HashWriteOptions {
            mode: HashWriteMode::Replace,
            ..Default::default()
        },
    )?;
    assert_eq!(
        client
            .hget(&key, "model-0001")?
            .ok_or("missing hash field")?
            .bytes,
        b"object-location"
    );
    assert!(client.del(&key)?.deleted);
    assert!(client.hget(&key, "model-0001")?.is_none());
    println!("PASS endpoint={endpoint} shm={shm} hset/hget/del and public error types");
    Ok(())
}
