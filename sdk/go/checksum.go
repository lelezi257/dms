package dms

import (
	"encoding/binary"
	"math/bits"
)

// payload 摘要是跨语言 wire 合同：Go/Rust 必须使用相同算法、seed 和字节序。
// XXH64 只做传输完整性校验，不用于鉴权或抵抗恶意碰撞。
const (
	xxhPrime1 uint64 = 11_400_714_785_074_694_791
	xxhPrime2 uint64 = 14_029_467_366_897_019_727
	xxhPrime3 uint64 = 1_609_587_929_392_839_161
	xxhPrime4 uint64 = 9_650_029_242_287_828_579
	xxhPrime5 uint64 = 2_870_177_450_012_600_261
)

func xxhRound(accumulator, input uint64) uint64 {
	return bits.RotateLeft64(accumulator+input*xxhPrime2, 31) * xxhPrime1
}

func xxhMergeRound(accumulator, value uint64) uint64 {
	return (accumulator ^ xxhRound(0, value))*xxhPrime1 + xxhPrime4
}

func stableDigestBytes(data []byte) []byte {
	offset := 0
	var hash uint64
	if len(data) >= 32 {
		lane1 := uint64(xxhPrime1)
		lane1 += xxhPrime2
		lane2 := xxhPrime2
		var lane3 uint64
		var lane4 uint64
		lane4 -= xxhPrime1
		for offset <= len(data)-32 {
			lane1 = xxhRound(lane1, binary.LittleEndian.Uint64(data[offset:]))
			lane2 = xxhRound(lane2, binary.LittleEndian.Uint64(data[offset+8:]))
			lane3 = xxhRound(lane3, binary.LittleEndian.Uint64(data[offset+16:]))
			lane4 = xxhRound(lane4, binary.LittleEndian.Uint64(data[offset+24:]))
			offset += 32
		}
		hash = bits.RotateLeft64(lane1, 1) + bits.RotateLeft64(lane2, 7) +
			bits.RotateLeft64(lane3, 12) + bits.RotateLeft64(lane4, 18)
		hash = xxhMergeRound(hash, lane1)
		hash = xxhMergeRound(hash, lane2)
		hash = xxhMergeRound(hash, lane3)
		hash = xxhMergeRound(hash, lane4)
	} else {
		hash = xxhPrime5
	}
	hash += uint64(len(data))
	for offset+8 <= len(data) {
		hash ^= xxhRound(0, binary.LittleEndian.Uint64(data[offset:]))
		hash = bits.RotateLeft64(hash, 27)*xxhPrime1 + xxhPrime4
		offset += 8
	}
	if offset+4 <= len(data) {
		hash ^= uint64(binary.LittleEndian.Uint32(data[offset:])) * xxhPrime1
		hash = bits.RotateLeft64(hash, 23)*xxhPrime2 + xxhPrime3
		offset += 4
	}
	for offset < len(data) {
		hash ^= uint64(data[offset]) * xxhPrime5
		hash = bits.RotateLeft64(hash, 11) * xxhPrime1
		offset++
	}
	hash ^= hash >> 33
	hash *= xxhPrime2
	hash ^= hash >> 29
	hash *= xxhPrime3
	hash ^= hash >> 32
	result := make([]byte, 8)
	binary.BigEndian.PutUint64(result, hash)
	return result
}
