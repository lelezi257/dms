package dms

import (
	"bytes"
	"context"
	"os"
	"testing"
	"time"
)

func TestIntegrationConnectOpensRealSession(t *testing.T) {
	endpoint := os.Getenv("DMS_GO_REAL_ENDPOINT")
	if endpoint == "" {
		t.Skip("DMS_GO_REAL_ENDPOINT is not set")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	client, err := Connect(ctx, endpoint, ClientOptions{Timeout: 2 * time.Second})
	if err != nil {
		t.Fatalf("Connect(%q) failed: %v", endpoint, err)
	}
	if client.currentSession().id == 0 {
		t.Fatal("Connect returned without opening a Worker session")
	}
	if err := client.Close(); err != nil {
		t.Fatalf("Close failed: %v", err)
	}
}

// TestIntegrationSetFromUsesRealSharedMemory 覆盖 inline 阈值以上的真实写路径：
// AllocateStaging -> AcquireRegion/SCM_RIGHTS -> mmap 填充 -> Set 提交。
//
// 这条路径与小对象 SetInline 完全不同，不能用“能建立 Session”或小对象读写代替验收。
// 测试默认跳过，只有显式提供本地 UDS Worker 时才执行，避免普通单元测试依赖外部进程。
func TestIntegrationSetFromUsesRealSharedMemory(t *testing.T) {
	endpoint := os.Getenv("DMS_GO_REAL_SHM_ENDPOINT")
	if endpoint == "" {
		t.Skip("DMS_GO_REAL_SHM_ENDPOINT is not set")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	shared := true
	client, err := Connect(ctx, endpoint, ClientOptions{
		Timeout:      5 * time.Second,
		SharedMemory: &shared,
	})
	if err != nil {
		t.Fatalf("Connect(%q) failed: %v", endpoint, err)
	}
	defer func() {
		if err := client.Close(); err != nil {
			t.Errorf("Close failed: %v", err)
		}
	}()

	value := bytes.Repeat([]byte("m1-staged-shm-"), 80*1024)
	key := "integration/go/set-from-shm"
	result, err := client.SetFrom(ctx, key, bytes.NewReader(value), uint64(len(value)), SetOptions{})
	if err != nil {
		t.Fatalf("SetFrom(%d bytes) failed: %v", len(value), err)
	}
	if result.Len != uint64(len(value)) {
		t.Fatalf("SetFrom length=%d, want %d", result.Len, len(value))
	}
	got, found, err := client.Get(ctx, key)
	if err != nil {
		t.Fatalf("Get after SetFrom failed: %v", err)
	}
	if !found || !bytes.Equal(got, value) {
		t.Fatalf("Get after SetFrom found=%v len=%d, want len=%d", found, len(got), len(value))
	}
}
