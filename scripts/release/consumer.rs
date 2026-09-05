//! 独立候选包消费者：只依赖 dms-client，不能引用源码树的内部 crate。
use dms_client::{ClientOptions, DmsClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = std::env::var("DMS_ENDPOINT")?;
    let shm = std::env::var("DMS_SHARED_MEMORY").as_deref() == Ok("true");
    let client = DmsClient::connect(
        &endpoint,
        ClientOptions { shared_memory: Some(shm), ..Default::default() },
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
    Ok(())
}
