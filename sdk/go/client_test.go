package dms

import (
	"bytes"
	"context"
	"errors"
	"io"
	"math"
	"net"
	"os"
	"strings"
	"sync"
	"testing"
	"time"

	pb "github.com/lelezi257/dms/sdk/go/internal/pb/dms/v1"
	"golang.org/x/sys/unix"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
)

func TestOptionsPrecedenceAndConstructors(t *testing.T) {
	t.Setenv("DMS_ENDPOINT", "127.0.0.1:1")
	t.Setenv("DMS_TIMEOUT_MILLIS", "100")
	shared := true
	inline := uint64(1024)
	resolved, err := resolveOptions("unix:///tmp/dms.sock", ClientOptions{
		Endpoint:             "127.0.0.1:2",
		Timeout:              2 * time.Second,
		InlineThresholdBytes: &inline,
		SharedMemory:         &shared,
	})
	if err != nil {
		t.Fatal(err)
	}
	if resolved.endpoint != "unix:///tmp/dms.sock" || resolved.timeout != 2*time.Second || resolved.inlineThresholdBytes != 1024 || !resolved.sharedMemory {
		t.Fatalf("unexpected resolved options: %+v", resolved)
	}
	if _, err := resolveOptions("", ClientOptions{TLS: "required"}); err == nil {
		t.Fatal("unsupported TLS must be rejected")
	}
}

func TestOptionsRejectOverflowAndAllowZeroInline(t *testing.T) {
	t.Setenv("DMS_ENDPOINT", "127.0.0.1:1")
	t.Setenv("DMS_INLINE_THRESHOLD_BYTES", "0")
	resolved, err := resolveOptions("", ClientOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if resolved.inlineThresholdBytes != 0 {
		t.Fatalf("inline threshold 0 should force staged path, got %d", resolved.inlineThresholdBytes)
	}

	t.Setenv("DMS_TIMEOUT_MILLIS", "9223372036855")
	if _, err := resolveOptions("", ClientOptions{}); !isKind(err, ErrorKindInvalidArgument) {
		t.Fatalf("overflowing timeout should be rejected, got %v", err)
	}
	t.Setenv("DMS_TIMEOUT_MILLIS", "100")
	t.Setenv("DMS_SESSION_CHANNEL_CAPACITY", "1048577")
	if _, err := resolveOptions("", ClientOptions{}); !isKind(err, ErrorKindInvalidArgument) {
		t.Fatalf("oversized session channel should be rejected, got %v", err)
	}

	t.Setenv("DMS_SESSION_CHANNEL_CAPACITY", "64")
	zero := uint64(0)
	resolved, err = resolveOptions("", ClientOptions{InlineThresholdBytes: &zero})
	if err != nil {
		t.Fatal(err)
	}
	if resolved.inlineThresholdBytes != 0 {
		t.Fatalf("explicit zero inline threshold not preserved: %d", resolved.inlineThresholdBytes)
	}
	tooLarge := uint64(maxSessionCapacity + 1)
	if _, err := resolveOptions("", ClientOptions{SessionChannelCapacity: &tooLarge}); !isKind(err, ErrorKindInvalidArgument) {
		t.Fatalf("oversized explicit session channel should be rejected, got %v", err)
	}
}

func TestPublicReadAndWriteConstructors(t *testing.T) {
	if ReadCurrent().exact != nil {
		t.Fatal("current read must not carry exact version")
	}
	exact := ReadExact(7)
	if exact.exact == nil || *exact.exact != 7 {
		t.Fatalf("bad exact version: %+v", exact)
	}
	if got := WriteIfVersion(9).wire(); got != "if-version:9" {
		t.Fatalf("bad if-version wire value %q", got)
	}
}

func TestSetGetDelUseNativeResultsAndThreeState(t *testing.T) {
	worker := &fakeWorker{getValue: []byte("value"), getVersion: 11}
	client := testClient(worker, nil)

	set, err := client.Set(context.Background(), "k", []byte("abc"))
	if err != nil || set.Version != 3 || set.Len != 3 {
		t.Fatalf("Set result=%+v err=%v", set, err)
	}
	got, found, err := client.Get(context.Background(), "k")
	if err != nil || !found || string(got) != "value" {
		t.Fatalf("Get got=%q found=%v err=%v", got, found, err)
	}
	got[0] = 'V'
	if string(worker.getValue) != "value" {
		t.Fatal("Get must return caller-owned bytes")
	}
	deleted, err := client.Del(context.Background(), "k")
	if err != nil || !deleted.Deleted || deleted.Version != 4 {
		t.Fatalf("Del result=%+v err=%v", deleted, err)
	}
	worker.getFound = false
	worker.getValue = nil
	_, found, err = client.Get(context.Background(), "missing")
	if err != nil || found {
		t.Fatalf("missing Get found=%v err=%v", found, err)
	}
}

func TestStagedUploadFailureCleanupIsBounded(t *testing.T) {
	worker := &fakeWorker{
		allocateTarget: &pb.PayloadTarget{},
		deleteContexts: make(chan error, 1),
		deleteBlock:    true,
	}
	client := testClient(worker, nil)
	client.inlineMax = 1
	client.timeout = 25 * time.Millisecond

	started := time.Now()
	_, err := client.Set(context.Background(), "large", []byte("larger-than-inline"))
	if err == nil {
		t.Fatal("expected upload failure")
	}
	var dmsErr *DmsError
	if !errors.As(err, &dmsErr) || dmsErr.Code != CLIENT_PROTOCOL_VIOLATION {
		t.Fatalf("cleanup must not mask original upload error code, got %T %v", err, err)
	}
	if !strings.Contains(err.Error(), "safe quarantine cleanup failed: delete staging after failed upload") {
		t.Fatalf("missing cleanup diagnostic in joined error: %v", err)
	}
	if elapsed := time.Since(started); elapsed > 500*time.Millisecond {
		t.Fatalf("bounded staging cleanup took too long: %s", elapsed)
	}
	if worker.deleteCount != 1 || worker.deleteStagingID != 99 {
		t.Fatalf("cleanup request mismatch count=%d staging=%d", worker.deleteCount, worker.deleteStagingID)
	}
	select {
	case ctxErr := <-worker.deleteContexts:
		if !errors.Is(ctxErr, context.DeadlineExceeded) {
			t.Fatalf("cleanup context ended with %v, want deadline", ctxErr)
		}
	default:
		t.Fatal("cleanup did not observe a bounded context deadline")
	}
}

func TestStagedShmSetEchoesReleaseTokenInReceipt(t *testing.T) {
	token := []byte("release-token")
	worker := &fakeWorker{
		allocateTarget: shmTarget(1, 0, 3, 77, token),
	}
	client := testClient(worker, nil)
	client.inlineMax = 1
	client.writeLeaseReleaseSupported = true
	client.regions[1] = &mappedRegion{id: 1, data: make([]byte, 3)}

	result, err := client.Set(context.Background(), "large", []byte("abc"))
	if err != nil || result.Version != 5 {
		t.Fatalf("Set result=%+v err=%v", result, err)
	}
	if worker.setReceipt == nil || !bytes.Equal(worker.setReceipt.ReleaseToken, token) || worker.setReceipt.TargetAllocationId != 77 {
		t.Fatalf("Set receipt did not echo release token: %+v", worker.setReceipt)
	}
	if worker.writeReleaseCount != 0 {
		t.Fatalf("successful Set should leave release handling to receipt path, heartbeat releases=%d", worker.writeReleaseCount)
	}
}

func TestStagedShmUploadFailureReleasesWriteLeaseBounded(t *testing.T) {
	token := []byte("release-token")
	worker := &fakeWorker{
		allocateTarget:    shmTarget(1, 3, 2, 77, token),
		deleteContexts:    make(chan error, 1),
		heartbeatContexts: make(chan error, 1),
		heartbeatBlock:    true,
	}
	client := testClient(worker, nil)
	client.inlineMax = 1
	client.timeout = 25 * time.Millisecond
	client.writeLeaseReleaseSupported = true
	client.regions[1] = &mappedRegion{id: 1, data: make([]byte, 3)}

	started := time.Now()
	_, err := client.Set(context.Background(), "large", []byte("abc"))
	if err == nil {
		t.Fatal("expected SHM upload failure")
	}
	var dmsErr *DmsError
	if !errors.As(err, &dmsErr) || dmsErr.Code != CLIENT_PROTOCOL_VIOLATION {
		t.Fatalf("release failure must not mask original upload error code, got %T %v", err, err)
	}
	if !strings.Contains(err.Error(), "safe quarantine cleanup failed: release write allocation 77") {
		t.Fatalf("missing write release cleanup diagnostic: %v", err)
	}
	if elapsed := time.Since(started); elapsed > 500*time.Millisecond {
		t.Fatalf("bounded write release took too long: %s", elapsed)
	}
	if worker.writeReleaseCount != 1 || worker.writeReleaseAllocation != 77 || !bytes.Equal(worker.writeReleaseToken, token) {
		t.Fatalf("write release mismatch count=%d allocation=%d token=%q", worker.writeReleaseCount, worker.writeReleaseAllocation, worker.writeReleaseToken)
	}
	select {
	case ctxErr := <-worker.heartbeatContexts:
		if !errors.Is(ctxErr, context.DeadlineExceeded) {
			t.Fatalf("release heartbeat context ended with %v, want deadline", ctxErr)
		}
	default:
		t.Fatal("release heartbeat did not observe bounded deadline")
	}
	if worker.deleteCount != 1 {
		t.Fatalf("pre-commit upload failure should still best-effort delete staging, count=%d", worker.deleteCount)
	}
}

func TestStagedShmUploadFailureOldServerDoesNotReleaseWriteLease(t *testing.T) {
	worker := &fakeWorker{
		allocateTarget: shmTarget(1, 3, 2, 77, []byte("release-token")),
	}
	client := testClient(worker, nil)
	client.inlineMax = 1
	client.writeLeaseReleaseSupported = false
	client.regions[1] = &mappedRegion{id: 1, data: make([]byte, 3)}

	_, err := client.Set(context.Background(), "large", []byte("abc"))
	if err == nil {
		t.Fatal("expected SHM upload failure")
	}
	if worker.writeReleaseCount != 0 {
		t.Fatalf("old server negotiation must not send write release heartbeat, count=%d", worker.writeReleaseCount)
	}
}

func TestStagedSetUnknownCommitDoesNotDeleteStaging(t *testing.T) {
	token := []byte("release-token")
	worker := &fakeWorker{
		allocateTarget: shmTarget(1, 0, 3, 77, token),
		setErr:         status.Error(codes.Unavailable, "unknown commit state"),
	}
	client := testClient(worker, nil)
	client.inlineMax = 1
	client.writeLeaseReleaseSupported = true
	client.regions[1] = &mappedRegion{id: 1, data: make([]byte, 3)}

	_, err := client.Set(context.Background(), "large", []byte("abc"))
	if !isKind(err, ErrorKindUnavailable) {
		t.Fatalf("Set error=%v, want unavailable", err)
	}
	if worker.deleteCount != 0 {
		t.Fatalf("unknown commit result must not delete staging, count=%d", worker.deleteCount)
	}
	if worker.writeReleaseCount != 1 || worker.writeReleaseAllocation != 77 || !bytes.Equal(worker.writeReleaseToken, token) {
		t.Fatalf("unknown commit write release mismatch count=%d allocation=%d token=%q", worker.writeReleaseCount, worker.writeReleaseAllocation, worker.writeReleaseToken)
	}
}

func TestStagedSetUnknownCommitReleaseFailurePreservesOriginalError(t *testing.T) {
	token := []byte("release-token")
	worker := &fakeWorker{
		allocateTarget:    shmTarget(1, 0, 3, 77, token),
		setErr:            status.Error(codes.Unavailable, "unknown commit state"),
		heartbeatContexts: make(chan error, 1),
		heartbeatBlock:    true,
	}
	client := testClient(worker, nil)
	client.inlineMax = 1
	client.timeout = 25 * time.Millisecond
	client.writeLeaseReleaseSupported = true
	client.regions[1] = &mappedRegion{id: 1, data: make([]byte, 3)}

	started := time.Now()
	_, err := client.Set(context.Background(), "large", []byte("abc"))
	if err == nil {
		t.Fatal("expected Set failure")
	}
	if elapsed := time.Since(started); elapsed > 500*time.Millisecond {
		t.Fatalf("bounded release cleanup took too long: %s", elapsed)
	}
	var dmsErr *DmsError
	if !errors.As(err, &dmsErr) || dmsErr.Code != CLIENT_CONNECTION_UNAVAILABLE || dmsErr.Kind != ErrorKindUnavailable {
		t.Fatalf("release cleanup must not mask original commit error, got %T %v", err, err)
	}
	if !strings.Contains(err.Error(), "unknown commit state") ||
		!strings.Contains(err.Error(), "safe quarantine cleanup failed: release write allocation 77") {
		t.Fatalf("joined error missing original or cleanup diagnostic: %v", err)
	}
	if worker.deleteCount != 0 {
		t.Fatalf("unknown commit result must not delete staging, count=%d", worker.deleteCount)
	}
	select {
	case ctxErr := <-worker.heartbeatContexts:
		if !errors.Is(ctxErr, context.DeadlineExceeded) {
			t.Fatalf("release heartbeat context ended with %v, want deadline", ctxErr)
		}
	default:
		t.Fatal("release heartbeat did not observe bounded deadline")
	}
}

func TestCanceledMapsToDeadlineExceededAndPreservesCause(t *testing.T) {
	err := asDmsError(context.Canceled)
	var dmsErr *DmsError
	if !errors.As(err, &dmsErr) || dmsErr.Kind != ErrorKindDeadlineExceeded || dmsErr.Code != CLIENT_DEADLINE_EXCEEDED {
		t.Fatalf("bad canceled mapping: %T %v", err, err)
	}
	if !errors.Is(err, context.Canceled) {
		t.Fatalf("mapped canceled error must preserve errors.Is context.Canceled: %v", err)
	}
	if kind := grpcKind(codes.Canceled); kind != ErrorKindDeadlineExceeded {
		t.Fatalf("grpc canceled kind=%s", kind)
	}
	err = asDmsError(status.Error(codes.Canceled, "client canceled"))
	if !errors.As(err, &dmsErr) || dmsErr.Kind != ErrorKindDeadlineExceeded || dmsErr.Code != CLIENT_DEADLINE_EXCEEDED {
		t.Fatalf("bad grpc canceled mapping: %+v", dmsErr)
	}
}

func TestSharedMemoryBoundsAndCloseCleanup(t *testing.T) {
	for _, p := range [][2]uint64{{7, 0}, {5, 2}, {math.MaxUint64, 2}, {1, math.MaxUint64}} {
		if _, err := copyOut([]byte("abcdef"), p[0], p[1]); err == nil {
			t.Fatalf("accepted invalid range %v", p)
		}
	}
	fd, err := unix.MemfdCreate("dms-go-sdk-close-test", unix.MFD_CLOEXEC)
	if err != nil {
		t.Fatal(err)
	}
	file := os.NewFile(uintptr(fd), "dms-go-sdk-close-test")
	defer file.Close()
	if err := unix.Ftruncate(fd, 4096); err != nil {
		t.Fatal(err)
	}
	data, err := unix.Mmap(fd, 0, 4096, unix.PROT_READ|unix.PROT_WRITE, unix.MAP_SHARED)
	if err != nil {
		t.Fatal(err)
	}
	mapped := true
	defer func() {
		if mapped {
			_ = unix.Munmap(data)
		}
	}()
	client := &Client{
		cancel:  func() {},
		regions: map[uint64]*mappedRegion{1: {id: 1, data: data, file: file}},
	}
	before, err := os.ReadFile("/proc/self/maps")
	if err != nil || !strings.Contains(string(before), "dms-go-sdk-close-test") {
		t.Fatalf("missing initial mapping: %v", err)
	}
	if err := client.Close(); err != nil {
		t.Fatal(err)
	}
	mapped = false
	after, err := os.ReadFile("/proc/self/maps")
	if err != nil || strings.Contains(string(after), "dms-go-sdk-close-test") {
		t.Fatalf("mapping survived Close: %v", err)
	}
	if _, err := file.Stat(); err == nil {
		t.Fatal("region fd still open")
	}
	if err := client.Close(); err != nil {
		t.Fatalf("Close must be idempotent: %v", err)
	}
}

func TestGetSortsSegmentsAndRejectsGaps(t *testing.T) {
	worker := &fakeWorker{
		getSegments: []*pb.ReadSegment{
			grpcSegment(3, []byte("def")),
			grpcSegment(0, []byte("abc")),
		},
		getVersion:       12,
		getLogicalLength: 6,
	}
	payload := &fakePayload{downloads: map[string][]byte{"a": []byte("abc"), "b": []byte("def")}}
	client := testClient(worker, payload)
	result, found, err := client.GetWithOptions(context.Background(), "k", GetOptions{})
	if err != nil || !found || string(result.Bytes) != "abcdef" {
		t.Fatalf("sorted read result=%q found=%v err=%v", result.Bytes, found, err)
	}

	worker.getSegments = []*pb.ReadSegment{grpcSegment(1, []byte("x"))}
	worker.getLogicalLength = 2
	_, _, err = client.GetWithOptions(context.Background(), "k", GetOptions{})
	var dmsErr *DmsError
	if !errors.As(err, &dmsErr) || dmsErr.Kind != ErrorKindInternal {
		t.Fatalf("gap must become protocol DmsError, got %T %v", err, err)
	}
}

func TestGetRangeInlineUsesRequestedLength(t *testing.T) {
	worker := &fakeWorker{
		getValue:         []byte("abcdefghi"),
		getVersion:       12,
		getLogicalLength: 4*1024*1024 + 1,
	}
	client := testClient(worker, nil)
	result, found, err := client.GetWithOptions(context.Background(), "k", GetOptions{
		Range: &ByteRange{Offset: 4*1024*1024 - 8, Len: 9},
	})
	if err != nil || !found || string(result.Bytes) != "abcdefghi" {
		t.Fatalf("range inline result=%q found=%v err=%v", result.Bytes, found, err)
	}
}

func TestGetRangeSegmentsUseRangeRelativeOffsets(t *testing.T) {
	worker := &fakeWorker{
		getSegments: []*pb.ReadSegment{
			grpcSegment(4, []byte("efghi")),
			grpcSegment(0, []byte("abcd")),
		},
		getVersion:       12,
		getLogicalLength: 4*1024*1024 + 1,
	}
	payload := &fakePayload{downloads: map[string][]byte{"a": []byte("abcd"), "b": []byte("efghi")}}
	client := testClient(worker, payload)
	result, found, err := client.GetWithOptions(context.Background(), "k", GetOptions{
		Range: &ByteRange{Offset: 4*1024*1024 - 8, Len: 9},
	})
	if err != nil || !found || string(result.Bytes) != "abcdefghi" {
		t.Fatalf("range segmented result=%q found=%v err=%v", result.Bytes, found, err)
	}
}

func TestNilPayloadDescriptorsBecomeProtocolErrors(t *testing.T) {
	client := testClient(&fakeWorker{}, nil)
	if _, err := client.decodeGet(context.Background(), &pb.GetResponse{Found: true, LogicalLength: 1, Segments: []*pb.ReadSegment{nil}}, nil, client.inlineMax, 1); !isKind(err, ErrorKindInternal) {
		t.Fatalf("nil read segment should be protocol error, got %v", err)
	}
	if _, err := client.upload(context.Background(), nil, []byte("x")); !isKind(err, ErrorKindInternal) {
		t.Fatalf("nil upload target should be protocol error, got %v", err)
	}
	if _, err := client.download(context.Background(), nil); !isKind(err, ErrorKindInternal) {
		t.Fatalf("nil download target should be protocol error, got %v", err)
	}
}

func TestAcquireRegionValidatesDescriptorBeforeRequestingFd(t *testing.T) {
	worker := &fakeWorker{
		acquireResp: &pb.AcquireRegionResponse{RegionId: 9, RegionLength: 4096, FdToken: []byte("token")},
	}
	client := testClient(worker, nil)
	client.fdPath = "/unused"
	_, err := client.mappingFor(context.Background(), &pb.ShmDescriptor{RegionId: 7, Offset: 0, Length: 1})
	if !isKind(err, ErrorKindInternal) {
		t.Fatalf("mismatched region should be protocol error, got %v", err)
	}

	worker.acquireResp = &pb.AcquireRegionResponse{RegionId: 7, RegionLength: 2, FdToken: []byte("token")}
	_, err = client.mappingFor(context.Background(), &pb.ShmDescriptor{RegionId: 7, Offset: 1, Length: 2})
	if !isKind(err, ErrorKindInternal) {
		t.Fatalf("out-of-range descriptor should be protocol error, got %v", err)
	}
}

func TestRequestFdReceivesCloseOnExecDescriptor(t *testing.T) {
	dir := t.TempDir()
	path := dir + "/broker.sock"
	listener, err := net.Listen("unix", path)
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()

	sourceFD, err := unix.MemfdCreate("dms-go-sdk-fd-cloexec-test", unix.MFD_CLOEXEC)
	if err != nil {
		t.Fatal(err)
	}
	defer unix.Close(sourceFD)

	serverDone := make(chan error, 1)
	go func() {
		conn, err := listener.Accept()
		if err != nil {
			serverDone <- err
			return
		}
		defer conn.Close()
		unixConn := conn.(*net.UnixConn)
		header := make([]byte, 4+8+8+4)
		if _, err := io.ReadFull(unixConn, header); err != nil {
			serverDone <- err
			return
		}
		if _, err := unixConn.Write([]byte{1}); err != nil {
			serverDone <- err
			return
		}
		_, _, err = unixConn.WriteMsgUnix([]byte{0}, unix.UnixRights(sourceFD), nil)
		serverDone <- err
	}()

	fd, err := requestFd(context.Background(), path, 1, 2, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer unix.Close(fd)
	if flags, err := unix.FcntlInt(uintptr(fd), unix.F_GETFD, 0); err != nil {
		t.Fatal(err)
	} else if flags&unix.FD_CLOEXEC == 0 {
		t.Fatalf("received fd missing FD_CLOEXEC: flags=%#x", flags)
	}
	if err := <-serverDone; err != nil {
		t.Fatal(err)
	}
}

func TestSharedReadReleasesEpochAfterWholeCopy(t *testing.T) {
	data := []byte("abcdef")
	firstEpoch := uint64(1)
	secondEpoch := uint64(2)
	worker := &fakeWorker{
		getSegments: []*pb.ReadSegment{
			{LogicalOffset: 0, Target: &pb.PayloadTarget{Target: &pb.PayloadTarget_Shm{Shm: &pb.ShmDescriptor{RegionId: 1, Offset: 0, Length: 3, ViewEpoch: &firstEpoch}}}},
			{LogicalOffset: 3, Target: &pb.PayloadTarget{Target: &pb.PayloadTarget_Shm{Shm: &pb.ShmDescriptor{RegionId: 1, Offset: 3, Length: 3, ViewEpoch: &secondEpoch}}}},
		},
		getVersion:       1,
		getLogicalLength: 6,
	}
	client := testClient(worker, nil)
	client.regions[1] = &mappedRegion{id: 1, data: data}
	got, found, err := client.Get(context.Background(), "k")
	if err != nil || !found || string(got) != "abcdef" {
		t.Fatalf("Get got=%q found=%v err=%v", got, found, err)
	}
	if worker.released != secondEpoch {
		t.Fatalf("expected release epoch %d, got %d", secondEpoch, worker.released)
	}
}

func TestViewReleaseTrackerDoesNotSkipDelayedEarlierGet(t *testing.T) {
	worker := &fakeWorker{}
	client := testClient(worker, nil)

	// GET2 先完成时只能记录 pending，不能把 released_through 直接跳到 2；
	// 否则仍在复制 GET1 的共享读保护会被 Node 过早回收。GET1 随后完成后才连续推进到 2。
	client.releaseViews(context.Background(), []uint64{2})
	if worker.released != 0 {
		t.Fatalf("out-of-order release skipped delayed epoch: got %d", worker.released)
	}
	client.releaseViews(context.Background(), []uint64{1})
	if worker.released != 2 {
		t.Fatalf("contiguous release did not advance through 2: got %d", worker.released)
	}
}

func TestReadRequestTrackerDoesNotSkipDelayedEarlierGet(t *testing.T) {
	firstStarted := make(chan struct{})
	secondStarted := make(chan struct{})
	releaseFirst := make(chan struct{})
	releaseSecond := make(chan struct{})
	worker := &fakeWorker{
		getFunc: func(ctx context.Context, req *pb.GetRequest) (*pb.GetResponse, error) {
			switch req.ReadRequestId {
			case 1:
				close(firstStarted)
				select {
				case <-releaseFirst:
				case <-ctx.Done():
					return nil, ctx.Err()
				}
			case 2:
				close(secondStarted)
				select {
				case <-releaseSecond:
				case <-ctx.Done():
					return nil, ctx.Err()
				}
			default:
				return nil, status.Errorf(codes.Internal, "unexpected read request id %d", req.ReadRequestId)
			}
			return inlineGetResponse(req.ReadRequestId, []byte("v")), nil
		},
	}
	client := testClient(worker, nil)

	firstDone := make(chan error, 1)
	go func() {
		_, _, err := client.Get(context.Background(), "k1")
		firstDone <- err
	}()
	<-firstStarted

	secondDone := make(chan error, 1)
	go func() {
		_, _, err := client.Get(context.Background(), "k2")
		secondDone <- err
	}()
	<-secondStarted

	close(releaseSecond)
	if err := <-secondDone; err != nil {
		t.Fatal(err)
	}
	client.sendUnaryHeartbeat(context.Background())
	if worker.finishedReadThrough != 0 {
		t.Fatalf("out-of-order read completion skipped delayed request: got %d", worker.finishedReadThrough)
	}

	close(releaseFirst)
	if err := <-firstDone; err != nil {
		t.Fatal(err)
	}
	client.sendUnaryHeartbeat(context.Background())
	if worker.finishedReadThrough != 2 {
		t.Fatalf("contiguous read completion did not advance through 2: got %d", worker.finishedReadThrough)
	}
}

func TestReadRequestFailureAdvancesContiguousGap(t *testing.T) {
	firstStarted := make(chan struct{})
	secondStarted := make(chan struct{})
	releaseFirst := make(chan struct{})
	releaseSecond := make(chan struct{})
	worker := &fakeWorker{
		getFunc: func(ctx context.Context, req *pb.GetRequest) (*pb.GetResponse, error) {
			switch req.ReadRequestId {
			case 1:
				close(firstStarted)
				select {
				case <-releaseFirst:
					return nil, status.Error(codes.Unavailable, "response lost")
				case <-ctx.Done():
					return nil, ctx.Err()
				}
			case 2:
				close(secondStarted)
				select {
				case <-releaseSecond:
					return inlineGetResponse(req.ReadRequestId, []byte("v")), nil
				case <-ctx.Done():
					return nil, ctx.Err()
				}
			default:
				return nil, status.Errorf(codes.Internal, "unexpected read request id %d", req.ReadRequestId)
			}
		},
	}
	client := testClient(worker, nil)

	firstDone := make(chan error, 1)
	go func() {
		_, _, err := client.Get(context.Background(), "k1")
		firstDone <- err
	}()
	<-firstStarted
	secondDone := make(chan error, 1)
	go func() {
		_, _, err := client.Get(context.Background(), "k2")
		secondDone <- err
	}()
	<-secondStarted

	close(releaseSecond)
	if err := <-secondDone; err != nil {
		t.Fatal(err)
	}
	client.sendUnaryHeartbeat(context.Background())
	if worker.finishedReadThrough != 0 {
		t.Fatalf("request 2 must stay pending while request 1 response is unresolved, got %d", worker.finishedReadThrough)
	}

	close(releaseFirst)
	if !isKind(<-firstDone, ErrorKindUnavailable) {
		t.Fatal("request 1 should return unavailable")
	}
	client.sendUnaryHeartbeat(context.Background())
	if worker.finishedReadThrough != 2 {
		t.Fatalf("failed request should close contiguous read gap through 2, got %d", worker.finishedReadThrough)
	}
}

func TestReadRequestCanceledBeforeArrivalCompletes(t *testing.T) {
	started := make(chan struct{})
	worker := &fakeWorker{
		getFunc: func(ctx context.Context, req *pb.GetRequest) (*pb.GetResponse, error) {
			if req.ReadRequestId != 1 {
				return nil, status.Errorf(codes.Internal, "unexpected read request id %d", req.ReadRequestId)
			}
			close(started)
			<-ctx.Done()
			return nil, ctx.Err()
		},
	}
	client := testClient(worker, nil)
	client.timeout = time.Second

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() {
		_, _, err := client.Get(ctx, "k")
		done <- err
	}()
	<-started
	cancel()
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Fatalf("canceled Get should preserve context.Canceled, got %v", err)
	}
	client.sendUnaryHeartbeat(context.Background())
	if worker.finishedReadThrough != 1 {
		t.Fatalf("canceled pre-arrival read request should complete through 1, got %d", worker.finishedReadThrough)
	}
}

func TestCloseFlushesFinishedReadRequestWatermark(t *testing.T) {
	worker := &fakeWorker{getValue: []byte("value")}
	client := testClient(worker, nil)

	got, found, err := client.Get(context.Background(), "k")
	if err != nil || !found || string(got) != "value" {
		t.Fatalf("Get got=%q found=%v err=%v", got, found, err)
	}
	if worker.finishedReadThrough != 0 {
		t.Fatalf("ordinary inline Get should not send a per-Get heartbeat, got %d", worker.finishedReadThrough)
	}
	if err := client.Close(); err != nil {
		t.Fatal(err)
	}
	if worker.finishedReadThrough != 1 {
		t.Fatalf("Close should flush finished read watermark through 1, got %d", worker.finishedReadThrough)
	}
}

func TestReadRequestEchoMismatchIsProtocolError(t *testing.T) {
	worker := &fakeWorker{
		getFunc: func(_ context.Context, req *pb.GetRequest) (*pb.GetResponse, error) {
			return inlineGetResponse(req.ReadRequestId+1, []byte("v")), nil
		},
	}
	client := testClient(worker, nil)
	if _, _, err := client.Get(context.Background(), "k"); !isKind(err, ErrorKindInternal) {
		t.Fatalf("mismatched GetResponse read id should be protocol error, got %v", err)
	}

	worker.getFunc = func(_ context.Context, req *pb.GetRequest) (*pb.GetResponse, error) {
		return &pb.GetResponse{
			Found:         true,
			Version:       1,
			LogicalLength: 1,
			Segments: []*pb.ReadSegment{{
				LogicalOffset: 0,
				ReadRequestId: req.ReadRequestId + 1,
				Target: &pb.PayloadTarget{Target: &pb.PayloadTarget_Grpc{Grpc: &pb.GrpcTarget{
					TransferId: []byte("a"),
					Length:     1,
				}}},
			}},
			ReadRequestId: req.ReadRequestId,
		}, nil
	}
	if _, _, err := client.Get(context.Background(), "k"); !isKind(err, ErrorKindInternal) {
		t.Fatalf("mismatched ReadSegment read id should be protocol error, got %v", err)
	}
}

func TestCloseWaitsForActiveGetBeforeUnmap(t *testing.T) {
	fd, err := unix.MemfdCreate("dms-go-sdk-close-race-test", unix.MFD_CLOEXEC)
	if err != nil {
		t.Fatal(err)
	}
	file := os.NewFile(uintptr(fd), "dms-go-sdk-close-race-test")
	if err := unix.Ftruncate(fd, 4096); err != nil {
		_ = file.Close()
		t.Fatal(err)
	}
	data, err := unix.Mmap(fd, 0, 4096, unix.PROT_READ|unix.PROT_WRITE, unix.MAP_SHARED)
	if err != nil {
		_ = file.Close()
		t.Fatal(err)
	}
	copy(data, []byte("abcdef"))
	worker := &fakeWorker{
		getStarted:       make(chan struct{}),
		getRelease:       make(chan struct{}),
		getSegments:      []*pb.ReadSegment{{LogicalOffset: 0, Target: &pb.PayloadTarget{Target: &pb.PayloadTarget_Shm{Shm: &pb.ShmDescriptor{RegionId: 1, Offset: 0, Length: 6}}}}},
		getVersion:       1,
		getLogicalLength: 6,
	}
	client := testClient(worker, nil)
	client.regions[1] = &mappedRegion{id: 1, data: data, file: file}

	getDone := make(chan error, 1)
	go func() {
		got, found, err := client.Get(context.Background(), "k")
		if err == nil && (!found || string(got) != "abcdef") {
			err = errors.New("active Get returned wrong bytes")
		}
		getDone <- err
	}()
	<-worker.getStarted

	closeDone := make(chan error, 1)
	go func() {
		closeDone <- client.Close()
	}()
	select {
	case err := <-closeDone:
		t.Fatalf("Close returned before active Get finished: %v", err)
	case <-time.After(30 * time.Millisecond):
	}

	close(worker.getRelease)
	if err := <-getDone; err != nil {
		t.Fatal(err)
	}
	if err := <-closeDone; err != nil {
		t.Fatal(err)
	}
}

func TestGrpcStatusDetailsMapToNativeDmsError(t *testing.T) {
	err := asDmsError(status.Error(codes.Unavailable, "node down"))
	var dmsErr *DmsError
	if !errors.As(err, &dmsErr) || dmsErr.Kind != ErrorKindUnavailable {
		t.Fatalf("bad mapped error: %T %v", err, err)
	}
	if dmsErr.Code != CLIENT_CONNECTION_UNAVAILABLE {
		t.Fatalf("fallback code must match Rust CLIENT_CONNECTION_UNAVAILABLE, got 0x%08x", dmsErr.Code)
	}

	withDetail, detailErr := status.New(codes.ResourceExhausted, "wrapped").WithDetails(&pb.ErrorDetail{
		DmsCode: 0x02010001,
		Kind:    pb.ErrorKind_ERROR_KIND_RESOURCE_EXHAUSTED,
		Message: "arena full",
	})
	if detailErr != nil {
		t.Fatal(detailErr)
	}
	err = asDmsError(withDetail.Err())
	if !errors.As(err, &dmsErr) {
		t.Fatalf("missing DmsError detail: %T %v", err, err)
	}
	if dmsErr.Code != 0x02010001 || dmsErr.Kind != ErrorKindResourceExhausted || dmsErr.Message != "arena full" {
		t.Fatalf("ErrorDetail golden mismatch: %+v", dmsErr)
	}
}

func TestStatAndScanUsePublicObjectInfo(t *testing.T) {
	modified := int64(123456789)
	worker := &fakeWorker{
		statFound: true,
		statInfo:  wireObjectInfo("k", 10, modified, 7),
		scanItems: []*pb.ObjectInfo{
			wireObjectInfo("p/1", 11, modified, 8),
			wireObjectInfo("p/2", 12, modified+1, 9),
		},
		scanNextCursor: "next",
	}
	client := testClient(worker, nil)
	info, found, err := client.Stat(context.Background(), "k")
	if err != nil || !found || info.Key != "k" || info.Len != 10 || info.Version != 7 || info.ModifiedTime.UnixMilli() != modified {
		t.Fatalf("Stat info=%+v found=%v err=%v", info, found, err)
	}

	start := "a"
	if _, err := client.Scan(context.Background(), "p", ScanOptions{StartAfter: &start, Cursor: "cursor"}); !isKind(err, ErrorKindInvalidArgument) {
		t.Fatalf("Scan mutually-exclusive options should fail locally, got %v", err)
	}
	scan, err := client.Scan(context.Background(), "p", ScanOptions{Limit: 2, StartAfter: &start})
	if err != nil || scan.NextCursor != "next" || len(scan.Items) != 2 {
		t.Fatalf("Scan result=%+v err=%v", scan, err)
	}
	if worker.scanLimit != 2 || worker.scanPrefix != "p" || worker.scanStartAfter != "a" {
		t.Fatalf("Scan request mismatch prefix=%q limit=%d start_after=%q", worker.scanPrefix, worker.scanLimit, worker.scanStartAfter)
	}
}

func wireObjectInfo(key string, length uint64, modifiedMillis int64, version uint64) *pb.ObjectInfo {
	return &pb.ObjectInfo{
		Key:                    &pb.Key{Value: []byte(key)},
		Length:                 length,
		ModifiedTimeUnixMillis: modifiedMillis,
		Version:                version,
	}
}

func shmTarget(regionID, offset, length, allocationID uint64, releaseToken []byte) *pb.PayloadTarget {
	return &pb.PayloadTarget{Target: &pb.PayloadTarget_Shm{Shm: &pb.ShmDescriptor{
		RegionId:     regionID,
		Offset:       offset,
		Length:       length,
		AllocationId: allocationID,
		TransferId:   []byte("shm-transfer"),
		ReleaseToken: append([]byte(nil), releaseToken...),
	}}}
}

func grpcSegment(offset uint64, payload []byte) *pb.ReadSegment {
	id := []byte("a")
	if offset != 0 {
		id = []byte("b")
	}
	return &pb.ReadSegment{
		LogicalOffset: offset,
		Target: &pb.PayloadTarget{Target: &pb.PayloadTarget_Grpc{Grpc: &pb.GrpcTarget{
			TransferId: id,
			Length:     uint64(len(payload)),
		}}},
	}
}

func inlineGetResponse(readRequestID uint64, value []byte) *pb.GetResponse {
	return &pb.GetResponse{
		Found:         true,
		Version:       1,
		LogicalLength: uint64(len(value)),
		InlineValue:   append([]byte(nil), value...),
		ReadRequestId: readRequestID,
	}
}

func testClient(worker *fakeWorker, payload *fakePayload) *Client {
	if payload == nil {
		payload = &fakePayload{}
	}
	return &Client{
		worker:    worker,
		payload:   payload,
		sessionID: 1,
		inlineMax: defaultInlineThreshold,
		timeout:   defaultTimeout,
		instance:  bytes.Repeat([]byte{1}, 16),
		cancel:    func() {},
		regions:   map[uint64]*mappedRegion{},
	}
}

func isKind(err error, kind ErrorKind) bool {
	var dmsErr *DmsError
	return errors.As(err, &dmsErr) && dmsErr.Kind == kind
}

type fakeWorker struct {
	pb.WorkerServiceClient
	mu                     sync.Mutex
	getValue               []byte
	getFound               bool
	getVersion             uint64
	getLogicalLength       uint64
	getSegments            []*pb.ReadSegment
	released               uint64
	getStarted             chan struct{}
	getRelease             chan struct{}
	getFunc                func(context.Context, *pb.GetRequest) (*pb.GetResponse, error)
	statFound              bool
	statInfo               *pb.ObjectInfo
	scanItems              []*pb.ObjectInfo
	scanNextCursor         string
	scanPrefix             string
	scanLimit              uint32
	scanStartAfter         string
	allocateTarget         *pb.PayloadTarget
	setErr                 error
	setReceipt             *pb.TransferReceipt
	deleteContexts         chan error
	deleteBlock            bool
	deleteCount            int
	deleteStagingID        uint64
	acquireResp            *pb.AcquireRegionResponse
	heartbeatContexts      chan error
	heartbeatBlock         bool
	writeReleaseCount      int
	writeReleaseAllocation uint64
	writeReleaseToken      []byte
	finishedReadThrough    uint64
}

func (w *fakeWorker) SetInline(context.Context, *pb.SetInlineRequest, ...grpc.CallOption) (*pb.SetResponse, error) {
	return &pb.SetResponse{Version: 3, Length: 3}, nil
}

func (w *fakeWorker) AllocateStaging(context.Context, *pb.AllocateStagingRequest, ...grpc.CallOption) (*pb.AllocateStagingResponse, error) {
	target := w.allocateTarget
	if target == nil {
		target = &pb.PayloadTarget{}
	}
	return &pb.AllocateStagingResponse{StagingId: 99, Target: target}, nil
}

func (w *fakeWorker) Set(_ context.Context, req *pb.SetRequest, _ ...grpc.CallOption) (*pb.SetResponse, error) {
	if req != nil && req.Value != nil && req.Value.Receipt != nil {
		w.mu.Lock()
		w.setReceipt = req.Value.Receipt
		w.mu.Unlock()
	}
	if w.setErr != nil {
		return nil, w.setErr
	}
	return &pb.SetResponse{Version: 5, Length: 3}, nil
}

func (w *fakeWorker) AcquireRegion(context.Context, *pb.AcquireRegionRequest, ...grpc.CallOption) (*pb.AcquireRegionResponse, error) {
	if w.acquireResp != nil {
		return w.acquireResp, nil
	}
	return &pb.AcquireRegionResponse{RegionId: 1, RegionLength: 4096}, nil
}

func (w *fakeWorker) Get(ctx context.Context, req *pb.GetRequest, _ ...grpc.CallOption) (*pb.GetResponse, error) {
	if w.getFunc != nil {
		return w.getFunc(ctx, req)
	}
	if w.getStarted != nil {
		close(w.getStarted)
	}
	if w.getRelease != nil {
		<-w.getRelease
	}
	w.mu.Lock()
	defer w.mu.Unlock()
	if !w.getFound && w.getValue == nil && w.getSegments == nil {
		return &pb.GetResponse{Found: false, ReadRequestId: req.ReadRequestId}, nil
	}
	if w.getSegments != nil {
		segments := make([]*pb.ReadSegment, 0, len(w.getSegments))
		for _, segment := range w.getSegments {
			if segment == nil {
				segments = append(segments, nil)
				continue
			}
			clone := proto.Clone(segment).(*pb.ReadSegment)
			clone.ReadRequestId = req.ReadRequestId
			segments = append(segments, clone)
		}
		return &pb.GetResponse{Found: true, Version: w.getVersion, LogicalLength: w.getLogicalLength, Segments: segments, ReadRequestId: req.ReadRequestId}, nil
	}
	value := append([]byte(nil), w.getValue...)
	logicalLength := w.getLogicalLength
	if logicalLength == 0 {
		logicalLength = uint64(len(value))
	}
	return &pb.GetResponse{Found: true, Version: w.getVersion, LogicalLength: logicalLength, InlineValue: value, ReadRequestId: req.ReadRequestId}, nil
}

func (w *fakeWorker) Delete(context.Context, *pb.DeleteRequest, ...grpc.CallOption) (*pb.DeleteResponse, error) {
	return &pb.DeleteResponse{Deleted: true, Version: 4}, nil
}

func (w *fakeWorker) DeleteStaging(ctx context.Context, req *pb.DeleteStagingRequest, _ ...grpc.CallOption) (*pb.DeleteStagingResponse, error) {
	w.mu.Lock()
	w.deleteCount++
	w.deleteStagingID = req.StagingId
	w.mu.Unlock()
	if w.deleteBlock {
		<-ctx.Done()
		if w.deleteContexts != nil {
			w.deleteContexts <- ctx.Err()
		}
		return nil, ctx.Err()
	}
	return &pb.DeleteStagingResponse{}, nil
}

func (w *fakeWorker) Stat(context.Context, *pb.StatRequest, ...grpc.CallOption) (*pb.StatResponse, error) {
	return &pb.StatResponse{Found: w.statFound, Info: w.statInfo}, nil
}

func (w *fakeWorker) Scan(_ context.Context, req *pb.ScanRequest, _ ...grpc.CallOption) (*pb.ScanResponse, error) {
	w.scanPrefix = string(req.Prefix)
	if req.Options != nil {
		w.scanLimit = req.Options.Limit
		w.scanStartAfter = string(req.Options.StartAfter)
	}
	return &pb.ScanResponse{Items: w.scanItems, NextCursor: w.scanNextCursor}, nil
}

func (w *fakeWorker) Heartbeat(ctx context.Context, req *pb.HeartbeatRequest, _ ...grpc.CallOption) (*pb.HeartbeatResponse, error) {
	w.mu.Lock()
	if req.ReleasedViewThrough != nil {
		w.released = *req.ReleasedViewThrough
	}
	if len(req.ReleasedWriteAllocations) > 0 {
		release := req.ReleasedWriteAllocations[0]
		w.writeReleaseCount++
		w.writeReleaseAllocation = release.AllocationId
		w.writeReleaseToken = append([]byte(nil), release.ReleaseToken...)
	}
	if req.FinishedReadRequestThrough != nil {
		w.finishedReadThrough = *req.FinishedReadRequestThrough
	}
	w.mu.Unlock()
	if w.heartbeatBlock {
		<-ctx.Done()
		if w.heartbeatContexts != nil {
			w.heartbeatContexts <- ctx.Err()
		}
		return nil, ctx.Err()
	}
	return &pb.HeartbeatResponse{}, nil
}

type fakePayload struct {
	pb.WorkerPayloadServiceClient
	downloads     map[string][]byte
	uploadReceipt *pb.TransferReceipt
}

func (p *fakePayload) Upload(context.Context, *pb.UploadPayloadRequest, ...grpc.CallOption) (*pb.UploadPayloadResponse, error) {
	return &pb.UploadPayloadResponse{Receipt: p.uploadReceipt}, nil
}

func (p *fakePayload) Download(_ context.Context, req *pb.DownloadPayloadRequest, _ ...grpc.CallOption) (*pb.DownloadPayloadResponse, error) {
	value := p.downloads[string(req.TransferId)]
	return &pb.DownloadPayloadResponse{Payload: append([]byte(nil), value...)}, nil
}
