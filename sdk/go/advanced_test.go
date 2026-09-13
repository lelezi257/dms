package dms

import (
	"context"
	"strings"
	"testing"

	pb "github.com/lelezi257/dms/sdk/go/internal/pb/dms/v1"
)

func TestAdvancedWritesUseConfiguredDurability(t *testing.T) {
	worker := &fakeWorker{allocateTarget: grpcTarget("advanced-write", 3)}
	client := testClient(worker, &fakePayload{})
	client.defaultDurability = DurabilityLocalMemory

	mset, err := client.MSet(context.Background(), []KVEntry{{Key: "a", Value: []byte("one")}, {Key: "b", Value: []byte("two")}}, MSetOptions{})
	if err != nil || len(mset.Versions) != 2 || mset.Versions[0].Key != "a" {
		t.Fatalf("MSet result=%+v err=%v", mset, err)
	}
	if worker.msetRequest.Durability != string(DurabilityLocalMemory) {
		t.Fatalf("MSet durability=%q", worker.msetRequest.Durability)
	}

	rangeResult, err := client.SetRange(context.Background(), "a", 4, []byte("xyz"))
	if err != nil || rangeResult.Version != 20 || rangeResult.Len != 7 {
		t.Fatalf("SetRange result=%+v err=%v", rangeResult, err)
	}
	if worker.setRangeRequest.Durability != string(DurabilityLocalMemory) || worker.setRangeRequest.Offset != 4 {
		t.Fatalf("SetRange request=%+v", worker.setRangeRequest)
	}

	hset, err := client.HSet(context.Background(), "hash", []HashEntry{{Field: "f", Value: []byte("abc")}}, HashWriteOptions{})
	if err != nil || hset.Version != 30 || hset.FieldCount != 1 {
		t.Fatalf("HSet result=%+v err=%v", hset, err)
	}
	if worker.hsetRequest.Mode != string(HashWriteMerge) || worker.hsetRequest.Durability != string(DurabilityLocalMemory) {
		t.Fatalf("HSet request=%+v", worker.hsetRequest)
	}

	hdel, err := client.HDel(context.Background(), "hash", []string{"f"}, HashDeleteOptions{})
	if err != nil || hdel.Version != 31 || worker.hdeleteRequest.Durability != string(DurabilityLocalMemory) {
		t.Fatalf("HDel result=%+v request=%+v err=%v", hdel, worker.hdeleteRequest, err)
	}

	hwrite, err := client.HWriteAt(context.Background(), "hash", "f", 5, []byte("xyz"), HashRangeWriteOptions{})
	if err != nil || hwrite.HashVersion != 33 || hwrite.ValueVersion != 34 || hwrite.Len != 8 {
		t.Fatalf("HWriteAt result=%+v err=%v", hwrite, err)
	}
	if worker.hwriteAtRequest.Durability != string(DurabilityLocalMemory) || worker.hwriteAtRequest.Offset != 5 {
		t.Fatalf("HWriteAt request=%+v", worker.hwriteAtRequest)
	}
}

func TestAdvancedReadsPreserveOrderAndHashVersions(t *testing.T) {
	hashVersion := uint64(41)
	worker := &fakeWorker{
		mgetResponse: &pb.MGetResponse{Items: []*pb.GetResponse{
			{Found: true, Version: 7, LogicalLength: 3, InlineValue: []byte("one")},
			{Found: false},
		}},
		hgetResponse: &pb.HGetResponse{Found: true, Value: hashWireValue("f1", "v1", hashVersion, 51)},
		hmgetResponse: &pb.HMGetResponse{HashVersion: &hashVersion, Values: []*pb.HGetResponse{
			{Found: true, Value: hashWireValue("f1", "v1", hashVersion, 51)},
			{Found: false},
		}},
		hgetAllResponse: &pb.HGetAllResponse{HashVersion: &hashVersion, Entries: []*pb.HashValueRead{
			hashWireValue("f1", "v1", hashVersion, 51),
			hashWireValue("f2", "v2", hashVersion, 52),
		}},
	}
	client := testClient(worker, nil)

	objects, err := client.MGet(context.Background(), []string{"a", "missing"})
	if err != nil || len(objects) != 2 || objects[0] == nil || string(objects[0].Bytes) != "one" || objects[1] != nil {
		t.Fatalf("MGet objects=%+v err=%v", objects, err)
	}

	value, found, err := client.HGet(context.Background(), "hash", "f1")
	if err != nil || !found || value.HashVersion != 41 || value.ValueVersion != 51 || string(value.Bytes) != "v1" {
		t.Fatalf("HGet value=%+v found=%v err=%v", value, found, err)
	}

	selected, err := client.HMGet(context.Background(), "hash", []string{"f1", "missing"}, HashGetOptions{})
	if err != nil || selected.Version == nil || *selected.Version != 41 || len(selected.Values) != 2 || selected.Values[0] == nil || selected.Values[1] != nil {
		t.Fatalf("HMGet result=%+v err=%v", selected, err)
	}

	all, err := client.HGetAll(context.Background(), "hash", HashGetOptions{})
	if err != nil || all.Version == nil || len(all.Entries) != 2 || all.Entries[1].Field != "f2" {
		t.Fatalf("HGetAll result=%+v err=%v", all, err)
	}

	scan, err := client.HScan(context.Background(), "hash", 9, HashScanOptions{})
	if err != nil || scan.Version == nil || *scan.Version != 32 || worker.hscanRequest.Cursor != 9 || worker.hscanRequest.Limit != defaultHashScanLimit {
		t.Fatalf("HScan result=%+v request=%+v err=%v", scan, worker.hscanRequest, err)
	}
}

func TestPublicStringBoundsFailBeforeRPC(t *testing.T) {
	worker := &fakeWorker{}
	client := testClient(worker, nil)
	overlongKey := strings.Repeat("k", MaxKeyLen+1)
	overlongField := strings.Repeat("f", MaxHashFieldLen+1)

	if _, err := client.Set(context.Background(), overlongKey, []byte("x")); !isKind(err, ErrorKindInvalidArgument) {
		t.Fatalf("overlong Set key err=%v", err)
	}
	if worker.setInlineCount != 0 {
		t.Fatal("invalid key reached Worker")
	}
	if _, _, err := client.HGet(context.Background(), "hash", overlongField); !isKind(err, ErrorKindInvalidArgument) {
		t.Fatalf("overlong Hash field err=%v", err)
	}
	for name, options := range map[string]ScanOptions{
		"prefix":     {},
		"startAfter": {StartAfter: &overlongKey},
		"delimiter":  {Delimiter: overlongKey},
	} {
		prefix := ""
		if name == "prefix" {
			prefix = overlongKey
		}
		if _, err := client.Scan(context.Background(), prefix, options); !isKind(err, ErrorKindInvalidArgument) {
			t.Fatalf("overlong %s err=%v", name, err)
		}
	}
	if worker.scanPrefix != "" {
		t.Fatal("invalid Scan input reached Worker")
	}
}

func TestUnavailableProvidersAndTLSAreStableUnimplemented(t *testing.T) {
	client := testClient(&fakeWorker{}, &fakePayload{})
	for name, target := range map[string]*pb.PayloadTarget{
		"rdma": {Target: &pb.PayloadTarget_Rdma{Rdma: &pb.RdmaTarget{Length: 1}}},
		"ub":   {Target: &pb.PayloadTarget_Ub{Ub: &pb.UbTarget{Length: 1}}},
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := readTargetLength(target); !isKind(err, ErrorKindUnimplemented) {
				t.Fatalf("read target err=%v", err)
			}
			if _, err := client.uploadForSession(context.Background(), client.currentSession(), target, []byte("x")); !isKind(err, ErrorKindUnimplemented) {
				t.Fatalf("upload err=%v", err)
			}
			if _, err := client.downloadForSession(context.Background(), client.currentSession(), target); !isKind(err, ErrorKindUnimplemented) {
				t.Fatalf("download err=%v", err)
			}
		})
	}
	if _, err := resolveOptions("127.0.0.1:1", ClientOptions{TLS: ClientTLSOptions("required")}); !isKind(err, ErrorKindUnimplemented) {
		t.Fatalf("TLS err=%v", err)
	}
}

func hashWireValue(field, value string, hashVersion, valueVersion uint64) *pb.HashValueRead {
	return &pb.HashValueRead{
		Field:         &pb.HashField{Value: []byte(field)},
		HashVersion:   hashVersion,
		ValueVersion:  valueVersion,
		LogicalLength: uint64(len(value)),
		InlineValue:   []byte(value),
	}
}

func uint64Ptr(value uint64) *uint64 {
	return &value
}
