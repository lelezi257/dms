package dms

import (
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
