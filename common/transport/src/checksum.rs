//! 当前 payload 完整性与请求幂等共用的稳定 64 位字节摘要。
//!
//! 这里只固定“相同 bytes 产生相同八字节结果”；字段拼接、版本与操作身份仍由业务
//! 决定。XXH64 不是密码学哈希，不用于鉴权、防恶意碰撞或证明远端数据真实存在。
//! 算法与 seed=0 在 0.1 协议中固定，升级时不能静默改成进程随机 Hasher。

const PRIME1: u64 = 11_400_714_785_074_694_791;
const PRIME2: u64 = 14_029_467_366_897_019_727;
const PRIME3: u64 = 1_609_587_929_392_839_161;
const PRIME4: u64 = 9_650_029_242_287_828_579;
const PRIME5: u64 = 2_870_177_450_012_600_261;

#[inline]
fn round(accumulator: u64, input: u64) -> u64 {
    accumulator
        .wrapping_add(input.wrapping_mul(PRIME2))
        .rotate_left(31)
        .wrapping_mul(PRIME1)
}

#[inline]
fn merge_round(accumulator: u64, value: u64) -> u64 {
    (accumulator ^ round(0, value))
        .wrapping_mul(PRIME1)
        .wrapping_add(PRIME4)
}

#[inline]
fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("caller checked eight-byte lane"),
    )
}

#[inline]
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("caller checked four-byte lane"),
    )
}

/// 稳定的 XXH64（seed=0），wire 仍使用八字节大端序。
pub fn stable_digest_bytes(bytes: &[u8]) -> [u8; 8] {
    let mut offset = 0;
    let mut hash = if bytes.len() >= 32 {
        let mut lane1 = PRIME1.wrapping_add(PRIME2);
        let mut lane2 = PRIME2;
        let mut lane3 = 0;
        let mut lane4 = 0_u64.wrapping_sub(PRIME1);
        while offset <= bytes.len() - 32 {
            lane1 = round(lane1, read_u64(bytes, offset));
            lane2 = round(lane2, read_u64(bytes, offset + 8));
            lane3 = round(lane3, read_u64(bytes, offset + 16));
            lane4 = round(lane4, read_u64(bytes, offset + 24));
            offset += 32;
        }
        let mut hash = lane1
            .rotate_left(1)
            .wrapping_add(lane2.rotate_left(7))
            .wrapping_add(lane3.rotate_left(12))
            .wrapping_add(lane4.rotate_left(18));
        hash = merge_round(hash, lane1);
        hash = merge_round(hash, lane2);
        hash = merge_round(hash, lane3);
        merge_round(hash, lane4)
    } else {
        PRIME5
    };
    hash = hash.wrapping_add(bytes.len() as u64);
    while offset + 8 <= bytes.len() {
        hash ^= round(0, read_u64(bytes, offset));
        hash = hash
            .rotate_left(27)
            .wrapping_mul(PRIME1)
            .wrapping_add(PRIME4);
        offset += 8;
    }
    if offset + 4 <= bytes.len() {
        hash ^= u64::from(read_u32(bytes, offset)).wrapping_mul(PRIME1);
        hash = hash
            .rotate_left(23)
            .wrapping_mul(PRIME2)
            .wrapping_add(PRIME3);
        offset += 4;
    }
    while offset < bytes.len() {
        hash ^= u64::from(bytes[offset]).wrapping_mul(PRIME5);
        hash = hash.rotate_left(11).wrapping_mul(PRIME1);
        offset += 1;
    }
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(PRIME2);
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(PRIME3);
    hash ^= hash >> 32;
    hash.to_be_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_digest_matches_xxh64_seed_zero_vectors() {
        assert_eq!(
            stable_digest_bytes(b""),
            0xef46db3751d8e999_u64.to_be_bytes()
        );
        assert_eq!(
            stable_digest_bytes(b"a"),
            0xd24ec4f1a98c6e5b_u64.to_be_bytes()
        );
        assert_eq!(
            stable_digest_bytes(b"abc"),
            0x44bc2cf5ad770999_u64.to_be_bytes()
        );
        assert_eq!(
            stable_digest_bytes(b"hello"),
            0x26c7827d889f6da3_u64.to_be_bytes()
        );
    }
}
