package dms

import (
	"encoding/hex"
	"fmt"
	"testing"
)

func TestStableDigestMatchesProtocolVectors(t *testing.T) {
	t.Parallel()
	for input, expected := range map[string]string{
		"":      "ef46db3751d8e999",
		"a":     "d24ec4f1a98c6e5b",
		"abc":   "44bc2cf5ad770999",
		"hello": "26c7827d889f6da3",
	} {
		actual := hex.EncodeToString(stableDigestBytes([]byte(input)))
		if actual != expected {
			t.Fatalf("digest(%q)=%s, want %s", input, actual, expected)
		}
	}
}

// BenchmarkStableDigestBytes 单独量化 Go SDK 在 SHM 写路径上的摘要成本。
// SET 端到端耗时包含 RPC、内存复制和 Meta 提交；保留这个原语基准，才能判断
// 大对象慢在传输还是慢在 SDK 本地完整性校验。
func BenchmarkStableDigestBytes(b *testing.B) {
	for _, size := range []int{4 << 10, 1 << 20} {
		b.Run(fmt.Sprintf("bytes-%d", size), func(b *testing.B) {
			payload := make([]byte, size)
			b.SetBytes(int64(size))
			b.ReportAllocs()
			b.ResetTimer()
			for range b.N {
				_ = stableDigestBytes(payload)
			}
		})
	}
}
