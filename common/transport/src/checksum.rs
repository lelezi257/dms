//! 当前 payload 完整性与请求幂等共用的字节摘要。
//!
//! 这里只固定“相同 bytes 产生相同八字节结果”；字段拼接、版本与操作身份仍由业务
//! 决定。FNV-1a 不是密码学哈希，不用于鉴权、防恶意碰撞或证明远端数据真实存在。

/// 与现有 receipt/WAL 兼容的 64-bit FNV-1a，大端字节序。
pub fn fnv1a_bytes(bytes: &[u8]) -> [u8; 8] {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash.to_be_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_receipt_bytes_are_stable() {
        assert_eq!(fnv1a_bytes(b""), 0xcbf29ce484222325_u64.to_be_bytes());
        assert_eq!(fnv1a_bytes(b"a"), 0xaf63dc4c8601ec8c_u64.to_be_bytes());
        assert_eq!(fnv1a_bytes(b"hello"), 0xa430d84680aabd0b_u64.to_be_bytes());
    }
}
