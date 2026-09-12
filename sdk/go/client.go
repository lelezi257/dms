package dms

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"math"
	"net"
	"net/url"
	"os"
	"sort"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	pb "github.com/lelezi257/dms/sdk/go/internal/pb/dms/v1"
	"golang.org/x/sys/unix"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

const protocolVersion = uint32(2)

// Client 是正式 Go SDK 的薄客户端。它复用连接、Session 和 Region mmap，但不跨请求
// 缓存 owned value；普通 Get 无论 TCP/SHM 都向 Node 读取并返回调用者自有 []byte。
type Client struct {
	conn    *grpc.ClientConn
	worker  pb.WorkerServiceClient
	payload pb.WorkerPayloadServiceClient

	sessionID                  uint64
	fdPath                     string
	inlineMax                  uint64
	timeout                    time.Duration
	writeLeaseReleaseSupported bool

	instance []byte
	nextSeq  atomic.Uint64

	ctx       context.Context
	cancel    context.CancelFunc
	closeOnce sync.Once
	closeErr  error
	closed    atomic.Bool
	streamWG  sync.WaitGroup
	activeWG  sync.WaitGroup

	mu              sync.Mutex
	regions         map[uint64]*mappedRegion
	lifecycleMu     sync.Mutex
	closing         bool
	viewReleases    viewReleaseTracker
	readRequests    readRequestTracker
	readReleaseWake chan struct{}
	readers         map[*objectReader]struct{}
	readerWG        sync.WaitGroup
}

// Connect 以显式 endpoint 建连；endpoint 覆盖环境变量和 ClientOptions.Endpoint。
func Connect(ctx context.Context, endpoint string, options ClientOptions) (*Client, error) {
	return connect(ctx, endpoint, options)
}

// ConnectWithOptions 从 options 或环境变量解析 endpoint。
func ConnectWithOptions(ctx context.Context, options ClientOptions) (*Client, error) {
	return connect(ctx, "", options)
}

func connect(ctx context.Context, endpoint string, options ClientOptions) (*Client, error) {
	resolved, err := resolveOptions(endpoint, options)
	if err != nil {
		return nil, &ConnectError{Err: asNative(err)}
	}
	callCtx, cancel := boundedContext(ctx, resolved.timeout)
	defer cancel()
	conn, err := dial(callCtx, resolved.endpoint)
	if err != nil {
		return nil, &ConnectError{Err: asNative(err)}
	}
	instance := make([]byte, 16)
	if _, err := rand.Read(instance); err != nil {
		_ = conn.Close()
		return nil, &ConnectError{Err: wrapDmsError(CLIENT_CONNECTION_UNAVAILABLE, ErrorKindInternal, "generate client instance id", err)}
	}
	clientCtx, clientCancel := context.WithCancel(context.Background())
	c := &Client{
		conn:            conn,
		worker:          pb.NewWorkerServiceClient(conn),
		payload:         pb.NewWorkerPayloadServiceClient(conn),
		inlineMax:       resolved.inlineThresholdBytes,
		timeout:         resolved.timeout,
		instance:        instance,
		ctx:             clientCtx,
		cancel:          clientCancel,
		regions:         map[uint64]*mappedRegion{},
		readReleaseWake: make(chan struct{}, 1),
	}
	open, err := c.worker.OpenSession(callCtx, &pb.OpenSessionRequest{
		MinVersion:                1,
		MaxVersion:                1,
		SharedMemory:              resolved.sharedMemory,
		ZeroCopyRead:              resolved.sharedMemory,
		ZeroCopyWrite:             resolved.sharedMemory,
		SupportsWriteLeaseRelease: true,
	})
	if err != nil {
		_ = conn.Close()
		clientCancel()
		return nil, &ConnectError{Err: asNative(err)}
	}
	c.sessionID = open.SessionId
	c.writeLeaseReleaseSupported = open.WriteLeaseReleaseSupported
	if open.Shm != nil {
		c.fdPath = open.Shm.FdBrokerPath
	}
	c.startSessionLoop(resolved.heartbeatInterval)
	return c, nil
}

func dial(ctx context.Context, endpoint string) (*grpc.ClientConn, error) {
	opts := []grpc.DialOption{
		grpc.WithTransportCredentials(insecure.NewCredentials()),
		grpc.WithBlock(),
		grpc.WithDefaultCallOptions(
			grpc.MaxCallRecvMsgSize(defaultMaxMessageBytes),
			grpc.MaxCallSendMsgSize(defaultMaxMessageBytes),
		),
	}
	if strings.HasPrefix(endpoint, "unix://") {
		path := strings.TrimPrefix(endpoint, "unix://")
		opts = append(opts, grpc.WithContextDialer(func(ctx context.Context, _ string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, "unix", path)
		}))
		return grpc.DialContext(ctx, "passthrough:///unix", opts...)
	}
	target := strings.TrimPrefix(endpoint, "tcp://")
	if parsed, err := url.Parse(target); err == nil && (parsed.Scheme == "http" || parsed.Scheme == "https") {
		if parsed.Scheme == "https" {
			return nil, invalidArgument("https endpoint requires TLS support, which is not implemented")
		}
		target = parsed.Host
	}
	return grpc.DialContext(ctx, target, opts...)
}

// Close 停止本地 goroutine 并释放 FD/mmap。当前 Worker wire 没有 CloseSession；
// 因此这里不虚构服务端同步释放，服务端资源仍由既有 Session/租约机制回收。
func (c *Client) Close() error {
	if c == nil {
		return nil
	}
	c.closeOnce.Do(func() {
		c.lifecycleMu.Lock()
		c.closing = true
		c.closed.Store(true)
		c.lifecycleMu.Unlock()
		c.activeWG.Wait()
		for _, reader := range c.snapshotReaders() {
			_ = reader.closeAndWait()
		}
		c.readerWG.Wait()
		c.sendUnaryHeartbeat(context.Background())
		c.cancel()
		c.streamWG.Wait()
		c.mu.Lock()
		for id, region := range c.regions {
			_ = unix.Munmap(region.data)
			if region.file != nil {
				_ = region.file.Close()
			}
			delete(c.regions, id)
		}
		c.mu.Unlock()
		if c.conn != nil {
			c.closeErr = c.conn.Close()
		}
	})
	return c.closeErr
}

func (c *Client) Set(ctx context.Context, key string, value []byte) (SetResult, error) {
	return c.SetWithOptions(ctx, key, value, SetOptions{})
}

func (c *Client) SetWithOptions(ctx context.Context, key string, value []byte, options SetOptions) (SetResult, error) {
	done, err := c.beginCall(key)
	if err != nil {
		return SetResult{}, err
	}
	defer done()
	if options.Durability == "" {
		options.Durability = DurabilityLocalMemory
	}
	if _, err := parseDurability("SetOptions.Durability", string(options.Durability)); err != nil {
		return SetResult{}, err
	}
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	if uint64(len(value)) <= c.inlineMax {
		resp, err := c.worker.SetInline(callCtx, &pb.SetInlineRequest{
			SessionId:   c.sessionID,
			Key:         &pb.Key{Value: []byte(key)},
			Value:       value,
			OperationId: c.nextOperation(),
			Condition:   options.Condition.wire(),
			Durability:  string(options.Durability),
		})
		if err != nil {
			return SetResult{}, asDmsError(err)
		}
		return SetResult{Version: ObjectVersion(resp.Version), Len: resp.Length}, nil
	}
	return c.setStaged(callCtx, key, value, options)
}

func (c *Client) SetFrom(ctx context.Context, key string, src io.Reader, length uint64, options SetOptions) (SetResult, error) {
	done, err := c.beginCall(key)
	if err != nil {
		return SetResult{}, err
	}
	defer done()
	if src == nil {
		return SetResult{}, invalidArgument("SetFrom source must be non-nil")
	}
	if options.Durability == "" {
		options.Durability = DurabilityLocalMemory
	}
	if _, err := parseDurability("SetOptions.Durability", string(options.Durability)); err != nil {
		return SetResult{}, err
	}
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	if length <= c.inlineMax {
		value, err := readExactBytes(src, length)
		if err != nil {
			return SetResult{}, err
		}
		resp, err := c.worker.SetInline(callCtx, &pb.SetInlineRequest{
			SessionId:   c.sessionID,
			Key:         &pb.Key{Value: []byte(key)},
			Value:       value,
			OperationId: c.nextOperation(),
			Condition:   options.Condition.wire(),
			Durability:  string(options.Durability),
		})
		if err != nil {
			return SetResult{}, asDmsError(err)
		}
		return SetResult{Version: ObjectVersion(resp.Version), Len: resp.Length}, nil
	}
	alloc, err := c.worker.AllocateStaging(callCtx, &pb.AllocateStagingRequest{
		SessionId: c.sessionID,
		Length:    length,
		Purpose:   "go-sdk-set-from",
	})
	if err != nil {
		return SetResult{}, asDmsError(err)
	}
	receipt, err := c.uploadFrom(callCtx, alloc.Target, src, length)
	if err != nil {
		return SetResult{}, setFailureWithCleanup(err, c.deleteStagingAfterUploadFailure(ctx, alloc.StagingId))
	}
	resp, err := c.worker.Set(callCtx, &pb.SetRequest{
		SessionId:   c.sessionID,
		Key:         &pb.Key{Value: []byte(key)},
		Value:       &pb.StagedValue{StagingId: alloc.StagingId, Receipt: receipt},
		OperationId: c.nextOperation(),
		Condition:   options.Condition.wire(),
		Durability:  string(options.Durability),
	})
	if err != nil {
		return SetResult{}, setFailureWithCleanup(err, c.releaseWriteFromReceipt(ctx, receipt))
	}
	return SetResult{Version: ObjectVersion(resp.Version), Len: resp.Length}, nil
}

func (c *Client) setStaged(ctx context.Context, key string, value []byte, options SetOptions) (SetResult, error) {
	alloc, err := c.worker.AllocateStaging(ctx, &pb.AllocateStagingRequest{
		SessionId: c.sessionID,
		Length:    uint64(len(value)),
		Purpose:   "go-sdk-set",
	})
	if err != nil {
		return SetResult{}, asDmsError(err)
	}
	receipt, err := c.upload(ctx, alloc.Target, value)
	if err != nil {
		// upload 失败发生在 Set 提交前，此时 staging 尚未被权威版本引用，可以做 best-effort 清理。
		// 清理不能继承调用方已取消的 ctx，否则会直接跳过；也不能无限等待，否则 Close 会被
		// activeWG 卡住。因此使用 SDK timeout 给 detached cleanup 明确边界。
		return SetResult{}, setFailureWithCleanup(err, c.deleteStagingAfterUploadFailure(ctx, alloc.StagingId))
	}
	resp, err := c.worker.Set(ctx, &pb.SetRequest{
		SessionId:   c.sessionID,
		Key:         &pb.Key{Value: []byte(key)},
		Value:       &pb.StagedValue{StagingId: alloc.StagingId, Receipt: receipt},
		OperationId: c.nextOperation(),
		Condition:   options.Condition.wire(),
		Durability:  string(options.Durability),
	})
	if err != nil {
		return SetResult{}, setFailureWithCleanup(err, c.releaseWriteFromReceipt(ctx, receipt))
	}
	return SetResult{Version: ObjectVersion(resp.Version), Len: resp.Length}, nil
}

func (c *Client) deleteStagingAfterUploadFailure(ctx context.Context, stagingID uint64) error {
	cleanupCtx, cleanupCancel := boundedContext(context.WithoutCancel(ctx), c.timeout)
	defer cleanupCancel()
	_, err := c.worker.DeleteStaging(cleanupCtx, &pb.DeleteStagingRequest{
		SessionId: c.sessionID,
		StagingId: stagingID,
	})
	return cleanupFailure("delete staging after failed upload", err)
}

func setFailureWithCleanup(primary error, cleanupErrs ...error) error {
	result := normalizeSetFailure(primary)
	for _, cleanupErr := range cleanupErrs {
		if cleanupErr != nil {
			result = errors.Join(result, cleanupErr)
		}
	}
	return result
}

func normalizeSetFailure(err error) error {
	var dmsErr *DmsError
	if errors.As(err, &dmsErr) {
		return err
	}
	return asDmsError(err)
}

func cleanupFailure(action string, err error) error {
	if err == nil {
		return nil
	}
	if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
		return wrapDmsError(CLIENT_DEADLINE_EXCEEDED, ErrorKindDeadlineExceeded, "safe quarantine cleanup failed: "+action+": "+err.Error(), err)
	}
	mapped := asNative(err)
	return &DmsError{
		Code:    mapped.Code,
		Kind:    mapped.Kind,
		Message: "safe quarantine cleanup failed: " + action + ": " + mapped.Message,
		Cause:   err,
	}
}

func (c *Client) Get(ctx context.Context, key string) ([]byte, bool, error) {
	result, found, err := c.GetWithOptions(ctx, key, GetOptions{})
	return result.Bytes, found, err
}

func (c *Client) GetWithOptions(ctx context.Context, key string, options GetOptions) (GetResult, bool, error) {
	plan, found, err := c.startRead(ctx, key, options)
	if err != nil {
		return GetResult{}, false, err
	}
	if !found {
		return GetResult{}, false, nil
	}
	defer plan.finish()
	out, err := c.decodeReadPlan(plan)
	if err != nil {
		return GetResult{}, false, asDmsError(err)
	}
	return GetResult{Version: plan.version, Bytes: out}, true, nil
}

func (c *Client) GetInto(ctx context.Context, key string, dst []byte, options GetOptions) (GetIntoResult, bool, error) {
	plan, found, err := c.startRead(ctx, key, options)
	if err != nil {
		return GetIntoResult{}, false, err
	}
	if !found {
		return GetIntoResult{}, false, nil
	}
	defer plan.finish()
	if uint64(len(dst)) < plan.length {
		return GetIntoResult{}, true, invalidArgument("GetInto destination buffer is smaller than selected read length")
	}
	if err := c.readPlanInto(plan, dst[:int(plan.length)]); err != nil {
		return GetIntoResult{}, true, asDmsError(err)
	}
	return GetIntoResult{Version: plan.version, Len: plan.length}, true, nil
}

func (c *Client) GetReader(ctx context.Context, key string, options GetOptions) (ReadResult, bool, error) {
	plan, found, err := c.startRead(ctx, key, options)
	if err != nil {
		return ReadResult{}, false, err
	}
	if !found {
		return ReadResult{}, false, nil
	}
	reader := newObjectReader(plan)
	if err := c.registerReader(reader); err != nil {
		plan.finish()
		return ReadResult{}, false, err
	}
	reader.armCancel()
	return ReadResult{Version: plan.version, Len: plan.length, Body: reader}, true, nil
}

func (c *Client) startRead(ctx context.Context, key string, options GetOptions) (*readPlan, bool, error) {
	done, err := c.beginCall(key)
	if err != nil {
		return nil, false, err
	}
	defer done()
	callCtx, cancel := boundedContext(ctx, c.timeout)
	readRequestID := c.readRequests.allocate()
	completed := false
	defer func() {
		if !completed {
			c.readRequests.complete(readRequestID)
			cancel()
		}
	}()
	req := &pb.GetRequest{
		SessionId:      c.sessionID,
		Key:            &pb.Key{Value: []byte(key)},
		MaxInlineBytes: c.inlineMax,
		ReadRequestId:  readRequestID,
		ClampRange:     options.ClampRange,
	}
	if options.Version.exact != nil {
		exact := uint64(*options.Version.exact)
		req.ExactVersion = &exact
	}
	if options.Range != nil {
		req.Range = &pb.ByteRange{Offset: options.Range.Offset, Length: options.Range.Len}
	}
	resp, err := c.worker.Get(callCtx, req)
	if err != nil {
		return nil, false, asDmsError(err)
	}
	if resp == nil {
		return nil, false, asDmsError(protocolError("Get response is empty"))
	}
	if resp.ReadRequestId != readRequestID {
		return nil, false, asDmsError(protocolError("Get response read request id mismatch"))
	}
	if !resp.Found {
		return nil, false, nil
	}
	releaseEpochs := readViewEpochs(resp.Segments)
	plan, err := c.newReadPlan(callCtx, cancel, resp, options.Range, options.ClampRange, c.inlineMax, readRequestID)
	if err != nil {
		c.releaseViews(context.WithoutCancel(callCtx), releaseEpochs)
		return nil, false, asDmsError(err)
	}
	completed = true
	return plan, true, nil
}

func (c *Client) Del(ctx context.Context, key string) (DeleteResult, error) {
	done, err := c.beginCall(key)
	if err != nil {
		return DeleteResult{}, err
	}
	defer done()
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	resp, err := c.worker.Delete(callCtx, &pb.DeleteRequest{
		SessionId:   c.sessionID,
		Key:         &pb.Key{Value: []byte(key)},
		OperationId: c.nextOperation(),
	})
	if err != nil {
		return DeleteResult{}, asDmsError(err)
	}
	return DeleteResult{Deleted: resp.Deleted, Version: ObjectVersion(resp.Version)}, nil
}

func (c *Client) Stat(ctx context.Context, key string) (ObjectInfo, bool, error) {
	done, err := c.beginCall(key)
	if err != nil {
		return ObjectInfo{}, false, err
	}
	defer done()
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	resp, err := c.worker.Stat(callCtx, &pb.StatRequest{
		SessionId: c.sessionID,
		Key:       &pb.Key{Value: []byte(key)},
	})
	if err != nil {
		return ObjectInfo{}, false, asDmsError(err)
	}
	if !resp.Found {
		return ObjectInfo{}, false, nil
	}
	info, err := objectInfoFromWire(resp.Info)
	if err != nil {
		return ObjectInfo{}, false, asDmsError(err)
	}
	return info, true, nil
}

func (c *Client) Scan(ctx context.Context, prefix string, options ScanOptions) (ScanResult, error) {
	done, err := c.beginClientCall()
	if err != nil {
		return ScanResult{}, err
	}
	defer done()
	if options.StartAfter != nil && options.Cursor != "" {
		return ScanResult{}, invalidArgument("ScanOptions.StartAfter and Cursor are mutually exclusive")
	}
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	wireOptions := &pb.ObjectScanOptions{
		Limit:     options.Limit,
		Cursor:    options.Cursor,
		Delimiter: []byte(options.Delimiter),
	}
	if options.StartAfter != nil {
		wireOptions.StartAfter = []byte(*options.StartAfter)
	}
	resp, err := c.worker.Scan(callCtx, &pb.ScanRequest{
		SessionId: c.sessionID,
		Prefix:    []byte(prefix),
		Options:   wireOptions,
	})
	if err != nil {
		return ScanResult{}, asDmsError(err)
	}
	items := make([]ObjectInfo, 0, len(resp.Items))
	for _, wire := range resp.Items {
		info, err := objectInfoFromWire(wire)
		if err != nil {
			return ScanResult{}, asDmsError(err)
		}
		items = append(items, info)
	}
	return ScanResult{Items: items, NextCursor: resp.NextCursor}, nil
}

func (c *Client) beginCall(key string) (func(), error) {
	if key == "" {
		return nil, invalidArgument("key must be non-empty")
	}
	return c.beginClientCall()
}

func (c *Client) beginClientCall() (func(), error) {
	if c == nil {
		return nil, invalidArgument("DMS client is closed")
	}
	c.lifecycleMu.Lock()
	defer c.lifecycleMu.Unlock()
	if c.closing || c.closed.Load() {
		return nil, invalidArgument("DMS client is closed")
	}
	c.activeWG.Add(1)
	return c.activeWG.Done, nil
}

type readPlan struct {
	client        *Client
	ctx           context.Context
	cancel        context.CancelFunc
	version       ObjectVersion
	length        uint64
	inlineValue   []byte
	segments      []*pb.ReadSegment
	releaseEpochs []uint64
	readRequestID uint64
	finishOnce    sync.Once
}

func (c *Client) newReadPlan(ctx context.Context, cancel context.CancelFunc, resp *pb.GetResponse, readRange *ByteRange, clampRange bool, maxInlineBytes uint64, readRequestID uint64) (*readPlan, error) {
	expectedLength, err := selectedReadLength(resp.LogicalLength, readRange, clampRange)
	if err != nil {
		return nil, err
	}
	plan := &readPlan{
		client:        c,
		ctx:           ctx,
		cancel:        cancel,
		version:       ObjectVersion(resp.Version),
		length:        expectedLength,
		releaseEpochs: readViewEpochs(resp.Segments),
		readRequestID: readRequestID,
	}
	if resp.InlineValue != nil {
		if maxInlineBytes == 0 {
			return nil, protocolError("inline read response is not allowed for this request")
		}
		if len(resp.Segments) != 0 {
			return nil, protocolError("inline read response must not also carry read segments")
		}
		if uint64(len(resp.InlineValue)) != expectedLength {
			return nil, protocolError("inline read response length does not match selected length")
		}
		if uint64(len(resp.InlineValue)) > maxInlineBytes {
			return nil, protocolError("inline read response exceeds requested budget")
		}
		plan.inlineValue = resp.InlineValue
		return plan, nil
	}
	segments := append([]*pb.ReadSegment(nil), resp.Segments...)
	for _, segment := range segments {
		if segment == nil || segment.Target == nil {
			return nil, protocolError("read segment has empty target")
		}
		if segment.ReadRequestId != readRequestID {
			return nil, protocolError("read segment request id does not match Get request")
		}
	}
	sort.Slice(segments, func(i, j int) bool {
		return segments[i].LogicalOffset < segments[j].LogicalOffset
	})
	var expectedOffset uint64
	for _, segment := range segments {
		if segment.LogicalOffset != expectedOffset {
			return nil, protocolError("read segments contain a gap or overlap")
		}
		length, err := readTargetLength(segment.Target)
		if err != nil {
			return nil, err
		}
		next, ok := checkedAdd(expectedOffset, length)
		if !ok {
			return nil, protocolError("read length overflow")
		}
		expectedOffset = next
	}
	if expectedOffset != expectedLength {
		return nil, protocolError("read segments length differs from selected length")
	}
	plan.segments = segments
	return plan, nil
}

func (p *readPlan) finish() {
	if p == nil {
		return
	}
	p.finishOnce.Do(func() {
		p.client.readRequests.complete(p.readRequestID)
		p.client.releaseViews(context.WithoutCancel(p.ctx), p.releaseEpochs)
		p.cancel()
	})
}

func (c *Client) decodeGet(ctx context.Context, resp *pb.GetResponse, readRange *ByteRange, maxInlineBytes uint64, readRequestID uint64) ([]byte, error) {
	readCtx, cancel := context.WithCancel(ctx)
	plan, err := c.newReadPlan(readCtx, cancel, resp, readRange, false, maxInlineBytes, readRequestID)
	if err != nil {
		return nil, err
	}
	defer cancel()
	return c.decodeReadPlan(plan)
}

func (c *Client) decodeReadPlan(plan *readPlan) ([]byte, error) {
	if plan.inlineValue != nil {
		// gRPC 为本次响应解码出独立 bytes；直接交给调用者，不再次复制。
		// 这里既不是跨请求缓存，也不借用 Node 的共享页。
		return plan.inlineValue, nil
	}
	// 完整校验布局后才取 payload，错误的后半段不能让前半段白白下载。
	// 首段已拥有 bytes（SHM 路径已复制一次），直接接管；多 Extent 仍顺序追加。
	// append 按需扩容，不相信远端长度预先申请无界内存，也不改变用户所有权。
	var out []byte
	for _, segment := range plan.segments {
		length, _ := readTargetLength(segment.Target) // 上面的完整预检已验证。
		part, err := c.download(plan.ctx, segment.Target)
		if err != nil {
			return nil, err
		}
		if uint64(len(part)) != length {
			return nil, protocolError("read payload length differs from descriptor")
		}
		if len(out) == 0 {
			out = part
		} else {
			out = append(out, part...)
		}
	}
	return out, nil
}

func (c *Client) readPlanInto(plan *readPlan, dst []byte) error {
	if plan.inlineValue != nil {
		copy(dst, plan.inlineValue)
		return nil
	}
	var written uint64
	for _, segment := range plan.segments {
		n, err := c.readSegmentInto(plan.ctx, segment.Target, dst[int(written):])
		if err != nil {
			return err
		}
		written += n
	}
	if written != plan.length {
		return protocolError("read payload length differs from descriptor")
	}
	return nil
}

func (c *Client) readSegmentInto(ctx context.Context, target *pb.PayloadTarget, dst []byte) (uint64, error) {
	if target == nil {
		return 0, protocolError("download target is empty")
	}
	switch t := target.GetTarget().(type) {
	case *pb.PayloadTarget_Shm:
		region, err := c.mappingFor(ctx, t.Shm)
		if err != nil {
			return 0, err
		}
		if err := copyOutInto(dst, region.data, t.Shm.Offset, t.Shm.Length); err != nil {
			return 0, err
		}
		return t.Shm.Length, nil
	case *pb.PayloadTarget_Grpc:
		resp, err := c.payload.Download(ctx, &pb.DownloadPayloadRequest{
			TransferId: t.Grpc.TransferId,
			Nonce:      t.Grpc.Nonce,
		})
		if err != nil {
			return 0, err
		}
		if resp == nil {
			return 0, protocolError("download response is empty")
		}
		if uint64(len(resp.Payload)) != t.Grpc.Length {
			return 0, protocolError("read payload length differs from descriptor")
		}
		copy(dst, resp.Payload)
		return t.Grpc.Length, nil
	default:
		return 0, protocolError("unsupported payload target")
	}
}

func selectedReadLength(logicalLength uint64, readRange *ByteRange, clampRange bool) (uint64, error) {
	if readRange == nil {
		return logicalLength, nil
	}
	if clampRange {
		if readRange.Offset >= logicalLength {
			return 0, nil
		}
		available := logicalLength - readRange.Offset
		if readRange.Len > available {
			return available, nil
		}
		return readRange.Len, nil
	}
	end, ok := checkedAdd(readRange.Offset, readRange.Len)
	if !ok || end > logicalLength {
		return 0, protocolError("read response logical length does not contain requested range")
	}
	return readRange.Len, nil
}

func (c *Client) startSessionLoop(interval time.Duration) {
	if interval <= 0 {
		interval = defaultHeartbeatInterval
	}
	c.streamWG.Add(1)
	go func() {
		defer c.streamWG.Done()
		for {
			if c.ctx.Err() != nil {
				return
			}
			if c.runSession(interval) == nil {
				return
			}
			timer := time.NewTimer(interval)
			select {
			case <-c.ctx.Done():
				timer.Stop()
				return
			case <-timer.C:
			}
		}
	}()
}

func (c *Client) runSession(interval time.Duration) error {
	// 单次连接拥有独立取消范围：CloseSend 只半关闭，发送失败时必须取消
	// 本次 Recv 才能进入重连；不能取消整个 Client 或丢弃累计归还水位。
	streamCtx, cancelStream := context.WithCancel(c.ctx)
	defer cancelStream()
	stream, err := c.worker.Session(streamCtx)
	if err != nil {
		c.sendUnaryHeartbeat(c.ctx)
		return err
	}
	errCh := make(chan error, 2)
	done := make(chan struct{})
	var wg sync.WaitGroup
	var sendMu sync.Mutex
	wg.Add(2)
	go func() {
		defer wg.Done()
		ticker := time.NewTicker(interval)
		defer ticker.Stop()
		// 首条消息标识 Session，同时重发重连前的累计完成水位。
		// 唤醒只合并“需要发送”这个信号；安全义务保存在 tracker，不存队列里。
		sendHeartbeat := func() error {
			sendMu.Lock()
			defer sendMu.Unlock()
			return stream.Send(&pb.ClientSessionMessage{
				SessionId: c.sessionID,
				Message: &pb.ClientSessionMessage_Heartbeat{Heartbeat: &pb.SessionHeartbeat{
					ReleasedViewThrough:        c.viewReleases.releasedViewThrough(),
					FinishedReadRequestThrough: c.readRequests.finishedReadRequestThrough(),
				}},
			})
		}
		if err := sendHeartbeat(); err != nil {
			errCh <- err
			return
		}
		for {
			select {
			case <-c.ctx.Done():
				errCh <- nil
				return
			case <-done:
				return
			case <-ticker.C:
			case <-c.readReleaseWake:
			}
			if err := sendHeartbeat(); err != nil {
				errCh <- err
				return
			}
		}
	}()
	go func() {
		defer wg.Done()
		for {
			event, err := stream.Recv()
			if err != nil {
				errCh <- err
				return
			}
			if event.EventSequence > 0 {
				sendMu.Lock()
				if err := stream.Send(&pb.ClientSessionMessage{
					SessionId: c.sessionID,
					Message: &pb.ClientSessionMessage_EventAck{EventAck: &pb.SessionEventAck{
						EventSequence: event.EventSequence,
					}},
				}); err != nil {
					sendMu.Unlock()
					errCh <- err
					return
				}
				sendMu.Unlock()
			}
		}
	}()
	select {
	case <-c.ctx.Done():
		close(done)
		cancelStream()
		_ = stream.CloseSend()
		wg.Wait()
		return nil
	case err := <-errCh:
		close(done)
		cancelStream()
		_ = stream.CloseSend()
		wg.Wait()
		return err
	}
}

func readTargetLength(target *pb.PayloadTarget) (uint64, error) {
	if target == nil {
		return 0, protocolError("read segment has empty target")
	}
	switch t := target.GetTarget().(type) {
	case *pb.PayloadTarget_Shm:
		if t.Shm == nil {
			return 0, protocolError("read segment has empty SHM descriptor")
		}
		if _, ok := checkedAdd(t.Shm.Offset, t.Shm.Length); !ok {
			return 0, protocolError("SHM descriptor range overflows")
		}
		return t.Shm.Length, nil
	case *pb.PayloadTarget_Grpc:
		if t.Grpc == nil {
			return 0, protocolError("read segment has empty gRPC descriptor")
		}
		return t.Grpc.Length, nil
	case *pb.PayloadTarget_Rdma, *pb.PayloadTarget_Ub:
		// 未实现的后端应在布局预检时失败，不能先下载其它片段再报错。
		return 0, protocolError("unsupported payload target")
	default:
		return 0, protocolError("read segment has empty target")
	}
}

func checkedAdd(left, right uint64) (uint64, bool) {
	sum := left + right
	return sum, sum >= left
}

func readViewEpochs(segments []*pb.ReadSegment) []uint64 {
	var epochs []uint64
	for _, segment := range segments {
		if segment == nil || segment.Target == nil {
			continue
		}
		if shm := segment.Target.GetShm(); shm != nil && shm.ViewEpoch != nil && *shm.ViewEpoch > 0 {
			epochs = append(epochs, *shm.ViewEpoch)
		}
	}
	return epochs
}

func objectInfoFromWire(info *pb.ObjectInfo) (ObjectInfo, error) {
	if info == nil || info.Key == nil {
		return ObjectInfo{}, protocolError("object info is missing key")
	}
	return ObjectInfo{
		Key:          string(info.Key.Value),
		Len:          info.Length,
		ModifiedTime: time.UnixMilli(info.ModifiedTimeUnixMillis),
		Version:      ObjectVersion(info.Version),
		IsPrefix:     info.IsPrefix,
	}, nil
}

type viewReleaseTracker struct {
	mu              sync.Mutex
	releasedThrough uint64
	pending         []viewInterval
}

type viewInterval struct {
	start uint64
	end   uint64
}

func (t *viewReleaseTracker) markReleased(epoch uint64) {
	if epoch == 0 {
		return
	}
	t.mu.Lock()
	defer t.mu.Unlock()
	if epoch <= t.releasedThrough {
		return
	}
	next := viewInterval{start: epoch, end: epoch}
	merged := make([]viewInterval, 0, len(t.pending)+1)
	for _, current := range t.pending {
		if epoch >= current.start && epoch <= current.end {
			return
		}
		if current.end != math.MaxUint64 && current.end+1 == next.start {
			next.start = current.start
			continue
		}
		if next.end != math.MaxUint64 && next.end+1 == current.start {
			next.end = current.end
			continue
		}
		merged = append(merged, current)
	}
	merged = append(merged, next)
	sort.Slice(merged, func(i, j int) bool { return merged[i].start < merged[j].start })
	t.pending = merged
	for len(t.pending) > 0 && t.releasedThrough != math.MaxUint64 && t.releasedThrough+1 == t.pending[0].start {
		t.releasedThrough = t.pending[0].end
		t.pending = t.pending[1:]
	}
}

func (t *viewReleaseTracker) releasedViewThrough() *uint64 {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.releasedThrough == 0 {
		return nil
	}
	value := t.releasedThrough
	return &value
}

type readRequestTracker struct {
	mu   sync.Mutex
	next uint64
	// 复用连续完成区间算法，但与 View 维护独立水位：二者生命周期不同。
	// 较早请求挂起时，后面十万次完成只占一个区间，不保留十万个 map 项。
	completed viewReleaseTracker
}

func (t *readRequestTracker) allocate() uint64 {
	t.mu.Lock()
	defer t.mu.Unlock()
	t.next++
	return t.next
}

func (t *readRequestTracker) complete(id uint64) {
	t.completed.markReleased(id)
}

func (t *readRequestTracker) finishedReadRequestThrough() *uint64 {
	return t.completed.releasedViewThrough()
}

func (c *Client) sendUnaryHeartbeat(ctx context.Context) {
	if c == nil || c.worker == nil {
		return
	}
	hctx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	_, _ = c.worker.Heartbeat(hctx, &pb.HeartbeatRequest{
		SessionId:                  c.sessionID,
		ReleasedViewThrough:        c.viewReleases.releasedViewThrough(),
		FinishedReadRequestThrough: c.readRequests.finishedReadRequestThrough(),
	})
}

func (c *Client) releaseViews(ctx context.Context, epochs []uint64) {
	if len(epochs) == 0 {
		return
	}
	for _, epoch := range epochs {
		c.viewReleases.markReleased(epoch)
	}
	if c.readReleaseWake == nil {
		// 未启动 Session 的内部测试/兼容调用继续同步归还；Connect 总会建立唤醒通道。
		c.sendUnaryHeartbeat(ctx)
		return
	}
	// 普通复制读已经结束，返回用户不必等待一个额外 Unary 往返。
	// 容量为1，仅合并唤醒，不丢水位；发送失败由后续心跳/重连/Close再次发送。
	select {
	case c.readReleaseWake <- struct{}{}:
	default:
	}
}

func (c *Client) registerReader(reader *objectReader) error {
	if c == nil || reader == nil {
		return invalidArgument("DMS client is closed")
	}
	c.lifecycleMu.Lock()
	defer c.lifecycleMu.Unlock()
	if c.closing || c.closed.Load() {
		return invalidArgument("DMS client is closed")
	}
	if c.readers == nil {
		c.readers = map[*objectReader]struct{}{}
	}
	c.readers[reader] = struct{}{}
	c.readerWG.Add(1)
	return nil
}

func (c *Client) unregisterReader(reader *objectReader) {
	if c == nil || reader == nil {
		return
	}
	c.lifecycleMu.Lock()
	if c.readers != nil {
		delete(c.readers, reader)
	}
	c.lifecycleMu.Unlock()
	c.readerWG.Done()
}

func (c *Client) snapshotReaders() []*objectReader {
	c.lifecycleMu.Lock()
	defer c.lifecycleMu.Unlock()
	readers := make([]*objectReader, 0, len(c.readers))
	for reader := range c.readers {
		readers = append(readers, reader)
	}
	return readers
}

type objectReader struct {
	plan *readPlan

	closeMu sync.Mutex
	closed  bool
	readWG  sync.WaitGroup

	inlineReader *bytes.Reader
	segmentIndex int
	segmentBytes []byte
	segmentPos   int

	cancelStop func() bool
	doneOnce   sync.Once
}

func newObjectReader(plan *readPlan) *objectReader {
	reader := &objectReader{plan: plan}
	if plan.inlineValue != nil {
		reader.inlineReader = bytes.NewReader(plan.inlineValue)
	}
	return reader
}

func (r *objectReader) armCancel() {
	r.cancelStop = context.AfterFunc(r.plan.ctx, func() {
		_ = r.closeAndWait()
	})
}

func (r *objectReader) Read(p []byte) (int, error) {
	if r == nil || r.plan == nil {
		return 0, io.ErrClosedPipe
	}
	r.closeMu.Lock()
	if r.closed {
		r.closeMu.Unlock()
		return 0, io.ErrClosedPipe
	}
	r.readWG.Add(1)
	r.closeMu.Unlock()

	n, err := r.readUnlocked(p)
	r.readWG.Done()
	if err == io.EOF {
		_ = r.closeNoWait()
	}
	return n, err
}

func (r *objectReader) readUnlocked(p []byte) (int, error) {
	if len(p) == 0 {
		return 0, nil
	}
	if err := r.plan.ctx.Err(); err != nil {
		return 0, err
	}
	if r.inlineReader != nil {
		return r.inlineReader.Read(p)
	}
	total := 0
	for total < len(p) {
		if len(r.segmentBytes) > r.segmentPos {
			n := copy(p[total:], r.segmentBytes[r.segmentPos:])
			r.segmentPos += n
			total += n
			if r.segmentPos == len(r.segmentBytes) {
				r.segmentBytes = nil
				r.segmentPos = 0
				r.segmentIndex++
			}
			continue
		}
		if r.segmentIndex >= len(r.plan.segments) {
			if total > 0 {
				return total, nil
			}
			return 0, io.EOF
		}
		segment := r.plan.segments[r.segmentIndex]
		switch target := segment.Target.GetTarget().(type) {
		case *pb.PayloadTarget_Shm:
			region, err := r.plan.client.mappingFor(r.plan.ctx, target.Shm)
			if err != nil {
				_ = r.closeNoWait()
				return total, err
			}
			remaining, err := shmSlice(region.data, target.Shm.Offset, target.Shm.Length)
			if err != nil {
				_ = r.closeNoWait()
				return total, err
			}
			if r.segmentPos > 0 {
				remaining = remaining[r.segmentPos:]
			}
			n := copy(p[total:], remaining)
			total += n
			r.segmentPos += n
			if uint64(r.segmentPos) == target.Shm.Length {
				r.segmentPos = 0
				r.segmentIndex++
			}
		case *pb.PayloadTarget_Grpc:
			resp, err := r.plan.client.payload.Download(r.plan.ctx, &pb.DownloadPayloadRequest{
				TransferId: target.Grpc.TransferId,
				Nonce:      target.Grpc.Nonce,
			})
			if err != nil {
				_ = r.closeNoWait()
				return total, err
			}
			if resp == nil {
				_ = r.closeNoWait()
				return total, protocolError("download response is empty")
			}
			if uint64(len(resp.Payload)) != target.Grpc.Length {
				_ = r.closeNoWait()
				return total, protocolError("read payload length differs from descriptor")
			}
			r.segmentBytes = resp.Payload
			r.segmentPos = 0
		default:
			_ = r.closeNoWait()
			return total, protocolError("unsupported payload target")
		}
	}
	return total, nil
}

func (r *objectReader) Close() error {
	return r.closeAndWait()
}

func (r *objectReader) closeNoWait() error {
	return r.close(false)
}

func (r *objectReader) closeAndWait() error {
	return r.close(true)
}

func (r *objectReader) close(wait bool) error {
	if r == nil || r.plan == nil {
		return nil
	}
	r.closeMu.Lock()
	alreadyClosed := r.closed
	r.closed = true
	r.closeMu.Unlock()
	if alreadyClosed {
		if wait {
			r.readWG.Wait()
		}
		return nil
	}
	if r.cancelStop != nil {
		r.cancelStop()
	}
	r.plan.cancel()
	if wait {
		r.readWG.Wait()
	}
	r.doneOnce.Do(func() {
		r.plan.finish()
		r.plan.client.unregisterReader(r)
	})
	return nil
}

func (c *Client) releaseWriteFromDescriptor(ctx context.Context, desc *pb.ShmDescriptor) error {
	if desc == nil {
		return nil
	}
	return c.releaseWriteAllocation(ctx, desc.AllocationId, desc.ReleaseToken)
}

func (c *Client) releaseWriteFromReceipt(ctx context.Context, receipt *pb.TransferReceipt) error {
	if receipt == nil {
		return nil
	}
	return c.releaseWriteAllocation(ctx, receipt.TargetAllocationId, receipt.ReleaseToken)
}

func (c *Client) releaseWriteAllocation(ctx context.Context, allocationID uint64, token []byte) error {
	if c == nil || !c.writeLeaseReleaseSupported || allocationID == 0 || len(token) == 0 {
		return nil
	}
	// 写租约释放只说明 SDK 已经退出本次共享写借用，不证明提交成功与否。
	// 旧 Node 未协商时不发送该字段；新 Node 使用有界 heartbeat 接收 token，
	// 避免依赖 TTL 回收 upload 失败或 unknown commit 后的已结束写权。
	release := &pb.ReleasedWriteAllocation{
		AllocationId: allocationID,
		ReleaseToken: append([]byte(nil), token...),
	}
	hctx, cancel := boundedContext(context.WithoutCancel(ctx), c.timeout)
	defer cancel()
	_, err := c.worker.Heartbeat(hctx, &pb.HeartbeatRequest{
		SessionId:                  c.sessionID,
		ReleasedWriteAllocations:   []*pb.ReleasedWriteAllocation{release},
		FinishedReadRequestThrough: c.readRequests.finishedReadRequestThrough(),
	})
	return cleanupFailure(fmt.Sprintf("release write allocation %d", allocationID), err)
}

func boundedContext(ctx context.Context, timeout time.Duration) (context.Context, context.CancelFunc) {
	if ctx == nil {
		ctx = context.Background()
	}
	if timeout <= 0 {
		timeout = defaultTimeout
	}
	return context.WithTimeout(ctx, timeout)
}

func asNative(err error) *DmsError {
	if err == nil {
		return nil
	}
	var dmsErr *DmsError
	if errors.As(err, &dmsErr) {
		return dmsErr
	}
	mapped := asDmsError(err)
	if errors.As(mapped, &dmsErr) {
		return dmsErr
	}
	return wrapDmsError(CLIENT_CONNECTION_UNAVAILABLE, ErrorKindUnavailable, err.Error(), err)
}

type mappedRegion struct {
	id   uint64
	data []byte
	file *os.File
}

func (c *Client) upload(ctx context.Context, target *pb.PayloadTarget, value []byte) (*pb.TransferReceipt, error) {
	if target == nil {
		return nil, protocolError("upload target is empty")
	}
	switch t := target.GetTarget().(type) {
	case *pb.PayloadTarget_Shm:
		region, err := c.mappingFor(ctx, t.Shm)
		if err != nil {
			return nil, setFailureWithCleanup(err, c.releaseWriteFromDescriptor(ctx, t.Shm))
		}
		if err := copyInto(region.data, t.Shm.Offset, value); err != nil {
			return nil, setFailureWithCleanup(err, c.releaseWriteFromDescriptor(ctx, t.Shm))
		}
		sum := stableDigestBytes(value)
		return &pb.TransferReceipt{
			TransferId:         t.Shm.TransferId,
			Length:             uint64(len(value)),
			Digest:             sum[:],
			TargetAllocationId: t.Shm.AllocationId,
			ReleaseToken:       append([]byte(nil), t.Shm.ReleaseToken...),
		}, nil
	case *pb.PayloadTarget_Grpc:
		resp, err := c.payload.Upload(ctx, &pb.UploadPayloadRequest{
			TransferId: t.Grpc.TransferId,
			Payload:    value,
			Nonce:      t.Grpc.Nonce,
		})
		if err != nil {
			return nil, err
		}
		if resp == nil || resp.Receipt == nil {
			return nil, protocolError("upload response is missing receipt")
		}
		return resp.Receipt, nil
	default:
		return nil, protocolError("unsupported payload target")
	}
}

func (c *Client) uploadFrom(ctx context.Context, target *pb.PayloadTarget, src io.Reader, length uint64) (*pb.TransferReceipt, error) {
	if target == nil {
		return nil, protocolError("upload target is empty")
	}
	switch t := target.GetTarget().(type) {
	case *pb.PayloadTarget_Shm:
		region, err := c.mappingFor(ctx, t.Shm)
		if err != nil {
			return nil, setFailureWithCleanup(err, c.releaseWriteFromDescriptor(ctx, t.Shm))
		}
		dst, err := shmSlice(region.data, t.Shm.Offset, length)
		if err != nil {
			return nil, setFailureWithCleanup(err, c.releaseWriteFromDescriptor(ctx, t.Shm))
		}
		if length != t.Shm.Length {
			return nil, setFailureWithCleanup(protocolError("SHM staging length differs from requested SetFrom length"), c.releaseWriteFromDescriptor(ctx, t.Shm))
		}
		if err := readFullInto(src, dst); err != nil {
			return nil, setFailureWithCleanup(err, c.releaseWriteFromDescriptor(ctx, t.Shm))
		}
		sum := stableDigestBytes(dst)
		return &pb.TransferReceipt{
			TransferId:         t.Shm.TransferId,
			Length:             length,
			Digest:             sum[:],
			TargetAllocationId: t.Shm.AllocationId,
			ReleaseToken:       append([]byte(nil), t.Shm.ReleaseToken...),
		}, nil
	case *pb.PayloadTarget_Grpc:
		// Current wire is unary protobuf bytes. SetFrom consumes exactly length
		// without reading past it, but TCP must buffer those bytes for the single
		// UploadPayloadRequest; this is protocol-required buffering, not a value cache.
		value, err := readExactBytes(src, length)
		if err != nil {
			return nil, err
		}
		resp, err := c.payload.Upload(ctx, &pb.UploadPayloadRequest{
			TransferId: t.Grpc.TransferId,
			Payload:    value,
			Nonce:      t.Grpc.Nonce,
		})
		if err != nil {
			return nil, err
		}
		if resp == nil || resp.Receipt == nil {
			return nil, protocolError("upload response is missing receipt")
		}
		return resp.Receipt, nil
	default:
		return nil, protocolError("unsupported payload target")
	}
}

func (c *Client) download(ctx context.Context, target *pb.PayloadTarget) ([]byte, error) {
	if target == nil {
		return nil, protocolError("download target is empty")
	}
	switch t := target.GetTarget().(type) {
	case *pb.PayloadTarget_Shm:
		region, err := c.mappingFor(ctx, t.Shm)
		if err != nil {
			return nil, err
		}
		return copyOut(region.data, t.Shm.Offset, t.Shm.Length)
	case *pb.PayloadTarget_Grpc:
		resp, err := c.payload.Download(ctx, &pb.DownloadPayloadRequest{
			TransferId: t.Grpc.TransferId,
			Nonce:      t.Grpc.Nonce,
		})
		if err != nil {
			return nil, err
		}
		return resp.Payload, nil
	default:
		return nil, protocolError("unsupported payload target")
	}
}

func (c *Client) mappingFor(ctx context.Context, desc *pb.ShmDescriptor) (*mappedRegion, error) {
	if desc == nil {
		return nil, protocolError("SHM descriptor is empty")
	}
	c.mu.Lock()
	if region := c.regions[desc.RegionId]; region != nil {
		c.mu.Unlock()
		return region, nil
	}
	c.mu.Unlock()
	if c.fdPath == "" {
		return nil, protocolError("SHM descriptor received without fd broker path")
	}
	resp, err := c.worker.AcquireRegion(ctx, &pb.AcquireRegionRequest{
		SessionId: c.sessionID,
		RegionId:  desc.RegionId,
	})
	if err != nil {
		return nil, err
	}
	if resp.RegionId != desc.RegionId || resp.RegionLength == 0 {
		return nil, protocolError("AcquireRegion returned a mismatched descriptor")
	}
	if desc.Offset > resp.RegionLength || desc.Length > resp.RegionLength-desc.Offset {
		return nil, protocolError("SHM descriptor exceeds acquired Region length")
	}
	maxInt := uint64(^uint(0) >> 1)
	if resp.RegionLength > maxInt {
		return nil, protocolError("SHM Region length is too large")
	}
	fd, err := requestFd(ctx, c.fdPath, c.sessionID, resp.RegionId, resp.FdToken)
	if err != nil {
		return nil, err
	}
	file := os.NewFile(uintptr(fd), "dms-shm-"+strconv.FormatUint(resp.RegionId, 10))
	data, err := unix.Mmap(int(file.Fd()), 0, int(resp.RegionLength), unix.PROT_READ|unix.PROT_WRITE, unix.MAP_SHARED)
	if err != nil {
		_ = file.Close()
		return nil, err
	}
	region := &mappedRegion{id: resp.RegionId, data: data, file: file}
	c.mu.Lock()
	defer c.mu.Unlock()
	if existing := c.regions[resp.RegionId]; existing != nil {
		_ = unix.Munmap(region.data)
		_ = region.file.Close()
		return existing, nil
	}
	c.regions[resp.RegionId] = region
	return region, nil
}

func requestFd(ctx context.Context, path string, sessionID, regionID uint64, token []byte) (int, error) {
	conn, err := (&net.Dialer{Timeout: 2 * time.Second}).DialContext(ctx, "unix", path)
	if err != nil {
		return -1, err
	}
	defer conn.Close()
	unixConn, ok := conn.(*net.UnixConn)
	if !ok {
		return -1, protocolError("fd broker connection is not unix")
	}
	_ = unixConn.SetDeadline(time.Now().Add(2 * time.Second))
	var req bytes.Buffer
	_ = binary.Write(&req, binary.BigEndian, protocolVersion)
	_ = binary.Write(&req, binary.BigEndian, sessionID)
	_ = binary.Write(&req, binary.BigEndian, regionID)
	_ = binary.Write(&req, binary.BigEndian, uint32(len(token)))
	req.Write(token)
	if _, err := unixConn.Write(req.Bytes()); err != nil {
		return -1, err
	}
	status := make([]byte, 1)
	if _, err := io.ReadFull(unixConn, status); err != nil {
		return -1, err
	}
	if status[0] != 1 {
		return -1, protocolError("fd broker rejected token")
	}
	buf := make([]byte, 1)
	oob := make([]byte, unix.CmsgSpace(4))
	syscall.ForkLock.RLock()
	defer syscall.ForkLock.RUnlock()
	_, oobn, _, _, err := unixConn.ReadMsgUnix(buf, oob)
	if err != nil {
		return -1, err
	}
	msgs, err := unix.ParseSocketControlMessage(oob[:oobn])
	if err != nil {
		return -1, err
	}
	for _, msg := range msgs {
		fds, err := unix.ParseUnixRights(&msg)
		if err != nil {
			return -1, err
		}
		if len(fds) > 0 {
			unix.CloseOnExec(fds[0])
			return fds[0], nil
		}
	}
	return -1, protocolError("missing SCM_RIGHTS fd")
}

func (c *Client) nextOperation() *pb.OperationId {
	return &pb.OperationId{
		ClientInstanceId: c.instance,
		Sequence:         c.nextSeq.Add(1),
	}
}

func copyInto(data []byte, offset uint64, value []byte) error {
	length := uint64(len(value))
	regionLength := uint64(len(data))
	if offset > regionLength || length > regionLength-offset {
		return protocolError(fmt.Sprintf("shm write out of range: offset=%d len=%d region=%d", offset, len(value), len(data)))
	}
	copy(data[offset:offset+length], value)
	return nil
}

func copyOut(data []byte, offset, length uint64) ([]byte, error) {
	if _, err := shmSlice(data, offset, length); err != nil {
		return nil, err
	}
	out := make([]byte, int(length))
	_ = copyOutInto(out, data, offset, length)
	return out, nil
}

func copyOutInto(dst, data []byte, offset, length uint64) error {
	src, err := shmSlice(data, offset, length)
	if err != nil {
		return err
	}
	if uint64(len(dst)) < length {
		return protocolError("destination buffer is smaller than SHM segment")
	}
	copy(dst[:int(length)], src)
	return nil
}

func shmSlice(data []byte, offset, length uint64) ([]byte, error) {
	regionLength := uint64(len(data))
	if offset > regionLength || length > regionLength-offset {
		return nil, protocolError(fmt.Sprintf("shm read out of range: offset=%d len=%d region=%d", offset, length, len(data)))
	}
	if length > uint64(int(^uint(0)>>1)) {
		return nil, protocolError("SHM slice length exceeds platform limit")
	}
	return data[int(offset):int(offset+length)], nil
}

func readExactBytes(src io.Reader, length uint64) ([]byte, error) {
	if length > uint64(int(^uint(0)>>1)) {
		return nil, invalidArgument("SetFrom length exceeds platform limit")
	}
	value := make([]byte, int(length))
	if err := readFullInto(src, value); err != nil {
		return nil, err
	}
	return value, nil
}

func readFullInto(src io.Reader, dst []byte) error {
	if len(dst) == 0 {
		return nil
	}
	if _, err := io.ReadFull(src, dst); err != nil {
		if errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) {
			return invalidArgument("SetFrom source ended before requested length")
		}
		return wrapDmsError(CLIENT_CONNECTION_UNAVAILABLE, ErrorKindUnavailable, "SetFrom source read failed: "+err.Error(), err)
	}
	return nil
}
