package dms

import (
	"context"
	"errors"

	pb "github.com/lelezi257/dms/sdk/go/internal/pb/dms/v1"
)

const defaultHashScanLimit = uint32(128)

type stagedUpload struct {
	id      uint64
	value   *pb.StagedValue
	receipt *pb.TransferReceipt
}

// MSet 原子发布多个独立 key。返回顺序与 entries 一致；空批次和重复 key 在发 RPC
// 前拒绝，避免把调用错误转成服务端状态错误。
func (c *Client) MSet(ctx context.Context, entries []KVEntry, options MSetOptions) (MSetResult, error) {
	if len(entries) == 0 {
		return MSetResult{}, invalidArgument("MSet entries must be non-empty")
	}
	seen := make(map[string]struct{}, len(entries))
	for _, entry := range entries {
		if err := validateKey(entry.Key); err != nil {
			return MSetResult{}, err
		}
		if _, exists := seen[entry.Key]; exists {
			return MSetResult{}, invalidArgument("MSet entries contain a duplicate key")
		}
		seen[entry.Key] = struct{}{}
	}
	done, err := c.beginClientCall()
	if err != nil {
		return MSetResult{}, err
	}
	defer done()
	durability, err := c.resolveDurability("MSetOptions.Durability", options.Durability)
	if err != nil {
		return MSetResult{}, err
	}
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	operationID := c.nextOperation()
	for attempt := 0; attempt < 2; attempt++ {
		session := c.currentSession()
		staged := make([]stagedUpload, 0, len(entries))
		wireEntries := make([]*pb.StagedKeyValue, 0, len(entries))
		stageFailed := false
		for _, entry := range entries {
			upload, stageErr := c.stageValue(callCtx, session, entry.Value, "go-sdk-mset-entry")
			if stageErr != nil {
				cleanupErr := c.cleanupStagedUploads(ctx, session, staged, true)
				if attempt == 0 && isSessionUnknown(stageErr) {
					if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
						return MSetResult{}, setFailureWithCleanup(reopenErr, cleanupErr)
					}
					stageFailed = true
					break
				}
				return MSetResult{}, setFailureWithCleanup(stageErr, cleanupErr)
			}
			staged = append(staged, upload)
			wireEntries = append(wireEntries, &pb.StagedKeyValue{
				Key:   &pb.Key{Value: []byte(entry.Key)},
				Value: upload.value,
			})
		}
		if stageFailed {
			continue
		}
		resp, rpcErr := c.worker.MSet(callCtx, &pb.MSetRequest{
			SessionId:   session.id,
			Entries:     wireEntries,
			OperationId: operationID,
			Durability:  string(durability),
		})
		if rpcErr == nil {
			return decodeMSetResult(resp, len(entries))
		}
		// 提交 RPC 一旦发出，普通错误可能是“已提交但回复丢失”；只能归还 SHM
		// 写权，不能删除可能已经被权威版本引用的 staging。
		cleanupErr := c.cleanupStagedUploads(ctx, session, staged, false)
		if attempt == 0 && isSessionUnknown(rpcErr) {
			cleanupErr = errors.Join(cleanupErr, c.deleteStagedUploads(ctx, session, staged))
			if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
				return MSetResult{}, setFailureWithCleanup(reopenErr, cleanupErr)
			}
			continue
		}
		return MSetResult{}, setFailureWithCleanup(rpcErr, cleanupErr)
	}
	return MSetResult{}, asDmsError(protocolError("MSet retry exhausted"))
}

// MGet 用一个 Worker 请求读取多个 key，并保留输入顺序。每个 nil 元素表示对应 key
// 不存在；整批共享同一个读请求生命周期，避免逐项提前释放共享页。
func (c *Client) MGet(ctx context.Context, keys []string) ([]*GetResult, error) {
	if len(keys) == 0 {
		return nil, invalidArgument("MGet keys must be non-empty")
	}
	wireKeys := make([]*pb.Key, 0, len(keys))
	for _, key := range keys {
		if err := validateKey(key); err != nil {
			return nil, err
		}
		wireKeys = append(wireKeys, &pb.Key{Value: []byte(key)})
	}
	done, err := c.beginClientCall()
	if err != nil {
		return nil, err
	}
	defer done()
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	readRequestID := c.readRequests.allocate()
	defer c.readRequests.complete(readRequestID)
	var resp *pb.MGetResponse
	session := c.currentSession()
	for attempt := 0; attempt < 2; attempt++ {
		resp, err = c.worker.MGet(callCtx, &pb.MGetRequest{
			SessionId:     session.id,
			Keys:          wireKeys,
			ReadRequestId: readRequestID,
		})
		if err == nil {
			break
		}
		if attempt == 0 && isSessionUnknown(err) {
			if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
				return nil, reopenErr
			}
			session = c.currentSession()
			continue
		}
		return nil, asDmsError(err)
	}
	if resp == nil || len(resp.Items) != len(keys) {
		return nil, asDmsError(protocolError("MGet response count does not match request"))
	}
	var releaseEpochs []uint64
	for _, item := range resp.Items {
		if item != nil {
			releaseEpochs = append(releaseEpochs, readViewEpochs(item.Segments)...)
		}
	}
	defer c.releaseViews(context.WithoutCancel(callCtx), releaseEpochs)
	results := make([]*GetResult, 0, len(resp.Items))
	for _, item := range resp.Items {
		if item == nil || item.ReadRequestId != readRequestID {
			return nil, asDmsError(protocolError("MGet item has an invalid read request id"))
		}
		if !item.Found {
			if item.InlineValue != nil || len(item.Segments) != 0 {
				return nil, asDmsError(protocolError("missing MGet item carries payload"))
			}
			results = append(results, nil)
			continue
		}
		plan, planErr := c.newReadPlan(callCtx, func() {}, item, nil, false, c.inlineMax, readRequestID, session)
		if planErr != nil {
			return nil, asDmsError(planErr)
		}
		bytes, decodeErr := c.decodeReadPlan(plan)
		if decodeErr != nil {
			return nil, asDmsError(decodeErr)
		}
		results = append(results, &GetResult{Version: plan.version, Bytes: bytes})
	}
	return results, nil
}

func (c *Client) SetRange(ctx context.Context, key string, offset uint64, data []byte) (SetResult, error) {
	return c.SetRangeWithOptions(ctx, key, offset, data, RangeWriteOptions{})
}

// SetRangeWithOptions 在一个已存在 value 上覆盖 [offset, offset+len(data))，并发布新版本。
func (c *Client) SetRangeWithOptions(ctx context.Context, key string, offset uint64, data []byte, options RangeWriteOptions) (SetResult, error) {
	done, err := c.beginCall(key)
	if err != nil {
		return SetResult{}, err
	}
	defer done()
	durability, err := c.resolveDurability("RangeWriteOptions.Durability", options.Durability)
	if err != nil {
		return SetResult{}, err
	}
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	operationID := c.nextOperation()
	for attempt := 0; attempt < 2; attempt++ {
		session := c.currentSession()
		staged, stageErr := c.stageValue(callCtx, session, data, "go-sdk-range-patch")
		if stageErr != nil {
			if attempt == 0 && isSessionUnknown(stageErr) {
				if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
					return SetResult{}, reopenErr
				}
				continue
			}
			return SetResult{}, asDmsError(stageErr)
		}
		resp, rpcErr := c.worker.SetRange(callCtx, &pb.SetRangeRequest{
			SessionId:       session.id,
			Key:             &pb.Key{Value: []byte(key)},
			Offset:          offset,
			Value:           staged.value,
			OperationId:     operationID,
			ExpectedVersion: objectVersionPtr(options.ExpectedVersion),
			Durability:      string(durability),
		})
		if rpcErr == nil {
			return SetResult{Version: ObjectVersion(resp.Version), Len: resp.Length}, nil
		}
		cleanupErr := c.cleanupStagedUploads(ctx, session, []stagedUpload{staged}, false)
		if attempt == 0 && isSessionUnknown(rpcErr) {
			cleanupErr = errors.Join(cleanupErr, c.deleteStagedUploads(ctx, session, []stagedUpload{staged}))
			if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
				return SetResult{}, setFailureWithCleanup(reopenErr, cleanupErr)
			}
			continue
		}
		return SetResult{}, setFailureWithCleanup(rpcErr, cleanupErr)
	}
	return SetResult{}, asDmsError(protocolError("SetRange retry exhausted"))
}

// HSet 原子更新同一 Hash/KKV 对象中的一个或多个字段。
func (c *Client) HSet(ctx context.Context, key string, entries []HashEntry, options HashWriteOptions) (HashSetResult, error) {
	if err := validateKey(key); err != nil {
		return HashSetResult{}, err
	}
	if len(entries) == 0 {
		return HashSetResult{}, invalidArgument("HSet entries must be non-empty")
	}
	seen := make(map[string]struct{}, len(entries))
	for _, entry := range entries {
		if err := validateHashField(entry.Field); err != nil {
			return HashSetResult{}, err
		}
		if _, exists := seen[entry.Field]; exists {
			return HashSetResult{}, invalidArgument("HSet entries contain a duplicate field")
		}
		seen[entry.Field] = struct{}{}
	}
	done, err := c.beginClientCall()
	if err != nil {
		return HashSetResult{}, err
	}
	defer done()
	mode := options.Mode
	if mode == "" {
		mode = HashWriteMerge
	}
	if mode != HashWriteMerge && mode != HashWriteReplace {
		return HashSetResult{}, invalidArgument("HashWriteOptions.Mode must be merge or replace")
	}
	durability, err := c.resolveDurability("HashWriteOptions.Durability", options.Durability)
	if err != nil {
		return HashSetResult{}, err
	}
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	operationID := c.nextOperation()
	for attempt := 0; attempt < 2; attempt++ {
		session := c.currentSession()
		staged := make([]stagedUpload, 0, len(entries))
		wireEntries := make([]*pb.StagedHashEntry, 0, len(entries))
		stageFailed := false
		for _, entry := range entries {
			upload, stageErr := c.stageValue(callCtx, session, entry.Value, "go-sdk-hash-field")
			if stageErr != nil {
				cleanupErr := c.cleanupStagedUploads(ctx, session, staged, true)
				if attempt == 0 && isSessionUnknown(stageErr) {
					if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
						return HashSetResult{}, setFailureWithCleanup(reopenErr, cleanupErr)
					}
					stageFailed = true
					break
				}
				return HashSetResult{}, setFailureWithCleanup(stageErr, cleanupErr)
			}
			staged = append(staged, upload)
			wireEntries = append(wireEntries, &pb.StagedHashEntry{
				Field: &pb.HashField{Value: []byte(entry.Field)},
				Value: upload.value,
			})
		}
		if stageFailed {
			continue
		}
		resp, rpcErr := c.worker.HSet(callCtx, &pb.HSetRequest{
			SessionId:       session.id,
			Key:             &pb.Key{Value: []byte(key)},
			Entries:         wireEntries,
			OperationId:     operationID,
			Mode:            string(mode),
			ExpectedVersion: hashVersionPtr(options.ExpectedVersion),
			Durability:      string(durability),
		})
		if rpcErr == nil {
			return HashSetResult{Version: HashVersion(resp.HashVersion), FieldCount: resp.FieldCount}, nil
		}
		cleanupErr := c.cleanupStagedUploads(ctx, session, staged, false)
		if attempt == 0 && isSessionUnknown(rpcErr) {
			cleanupErr = errors.Join(cleanupErr, c.deleteStagedUploads(ctx, session, staged))
			if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
				return HashSetResult{}, setFailureWithCleanup(reopenErr, cleanupErr)
			}
			continue
		}
		return HashSetResult{}, setFailureWithCleanup(rpcErr, cleanupErr)
	}
	return HashSetResult{}, asDmsError(protocolError("HSet retry exhausted"))
}

func (c *Client) HGet(ctx context.Context, key, field string) (HashValue, bool, error) {
	return c.HGetWithOptions(ctx, key, field, HashGetOptions{})
}

func (c *Client) HGetWithOptions(ctx context.Context, key, field string, options HashGetOptions) (HashValue, bool, error) {
	if err := validateHashField(field); err != nil {
		return HashValue{}, false, err
	}
	done, err := c.beginCall(key)
	if err != nil {
		return HashValue{}, false, err
	}
	defer done()
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	readID := c.readRequests.allocate()
	defer c.readRequests.complete(readID)
	var resp *pb.HGetResponse
	for attempt := 0; attempt < 2; attempt++ {
		session := c.currentSession()
		resp, err = c.worker.HGet(callCtx, &pb.HGetRequest{
			SessionId:        session.id,
			Key:              &pb.Key{Value: []byte(key)},
			Field:            &pb.HashField{Value: []byte(field)},
			ExactHashVersion: hashReadVersionPtr(options.Version),
			ReadRequestId:    readID,
		})
		if err == nil {
			break
		}
		if attempt == 0 && isSessionUnknown(err) {
			if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
				return HashValue{}, false, reopenErr
			}
			continue
		}
		return HashValue{}, false, asDmsError(err)
	}
	return decodeHashGet(resp)
}

func (c *Client) HMGet(ctx context.Context, key string, fields []string, options HashGetOptions) (HashMultiGetResult, error) {
	if len(fields) == 0 {
		return HashMultiGetResult{}, invalidArgument("HMGet fields must be non-empty")
	}
	wireFields := make([]*pb.HashField, 0, len(fields))
	for _, field := range fields {
		if err := validateHashField(field); err != nil {
			return HashMultiGetResult{}, err
		}
		wireFields = append(wireFields, &pb.HashField{Value: []byte(field)})
	}
	done, err := c.beginCall(key)
	if err != nil {
		return HashMultiGetResult{}, err
	}
	defer done()
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	readID := c.readRequests.allocate()
	defer c.readRequests.complete(readID)
	var resp *pb.HMGetResponse
	for attempt := 0; attempt < 2; attempt++ {
		session := c.currentSession()
		resp, err = c.worker.HMGet(callCtx, &pb.HMGetRequest{
			SessionId:        session.id,
			Key:              &pb.Key{Value: []byte(key)},
			Fields:           wireFields,
			ExactHashVersion: hashReadVersionPtr(options.Version),
			ReadRequestId:    readID,
		})
		if err == nil {
			break
		}
		if attempt == 0 && isSessionUnknown(err) {
			if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
				return HashMultiGetResult{}, reopenErr
			}
			continue
		}
		return HashMultiGetResult{}, asDmsError(err)
	}
	if resp == nil || len(resp.Values) != len(fields) {
		return HashMultiGetResult{}, asDmsError(protocolError("HMGet response count does not match request"))
	}
	result := HashMultiGetResult{Version: optionalHashVersion(resp.HashVersion), Values: make([]*HashValue, 0, len(resp.Values))}
	for _, item := range resp.Values {
		value, found, decodeErr := decodeHashGet(item)
		if decodeErr != nil {
			return HashMultiGetResult{}, decodeErr
		}
		if !found {
			result.Values = append(result.Values, nil)
		} else {
			copyValue := value
			result.Values = append(result.Values, &copyValue)
		}
	}
	return result, nil
}

func (c *Client) HGetAll(ctx context.Context, key string, options HashGetOptions) (HashEntriesResult, error) {
	done, err := c.beginCall(key)
	if err != nil {
		return HashEntriesResult{}, err
	}
	defer done()
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	readID := c.readRequests.allocate()
	defer c.readRequests.complete(readID)
	var resp *pb.HGetAllResponse
	for attempt := 0; attempt < 2; attempt++ {
		session := c.currentSession()
		resp, err = c.worker.HGetAll(callCtx, &pb.HGetAllRequest{
			SessionId:        session.id,
			Key:              &pb.Key{Value: []byte(key)},
			ExactHashVersion: hashReadVersionPtr(options.Version),
			ReadRequestId:    readID,
		})
		if err == nil {
			break
		}
		if attempt == 0 && isSessionUnknown(err) {
			if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
				return HashEntriesResult{}, reopenErr
			}
			continue
		}
		return HashEntriesResult{}, asDmsError(err)
	}
	entries, err := decodeHashValues(resp.GetEntries())
	if err != nil {
		return HashEntriesResult{}, err
	}
	return HashEntriesResult{Version: optionalHashVersion(resp.HashVersion), Entries: entries}, nil
}

func (c *Client) HDel(ctx context.Context, key string, fields []string, options HashDeleteOptions) (HashSetResult, error) {
	if len(fields) == 0 {
		return HashSetResult{}, invalidArgument("HDel fields must be non-empty")
	}
	wireFields := make([]*pb.HashField, 0, len(fields))
	seen := make(map[string]struct{}, len(fields))
	for _, field := range fields {
		if err := validateHashField(field); err != nil {
			return HashSetResult{}, err
		}
		if _, exists := seen[field]; exists {
			return HashSetResult{}, invalidArgument("HDel fields contain a duplicate field")
		}
		seen[field] = struct{}{}
		wireFields = append(wireFields, &pb.HashField{Value: []byte(field)})
	}
	done, err := c.beginCall(key)
	if err != nil {
		return HashSetResult{}, err
	}
	defer done()
	durability, err := c.resolveDurability("HashDeleteOptions.Durability", options.Durability)
	if err != nil {
		return HashSetResult{}, err
	}
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	operationID := c.nextOperation()
	for attempt := 0; attempt < 2; attempt++ {
		session := c.currentSession()
		resp, rpcErr := c.worker.HDelete(callCtx, &pb.HDeleteRequest{
			SessionId:       session.id,
			Key:             &pb.Key{Value: []byte(key)},
			Fields:          wireFields,
			OperationId:     operationID,
			ExpectedVersion: hashVersionPtr(options.ExpectedVersion),
			Durability:      string(durability),
		})
		if rpcErr == nil {
			return HashSetResult{Version: HashVersion(resp.HashVersion), FieldCount: resp.FieldCount}, nil
		}
		if attempt == 0 && isSessionUnknown(rpcErr) {
			if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
				return HashSetResult{}, reopenErr
			}
			continue
		}
		return HashSetResult{}, asDmsError(rpcErr)
	}
	return HashSetResult{}, asDmsError(protocolError("HDel retry exhausted"))
}

func (c *Client) HScan(ctx context.Context, key string, cursor ScanCursor, options HashScanOptions) (HashScanResult, error) {
	done, err := c.beginCall(key)
	if err != nil {
		return HashScanResult{}, err
	}
	defer done()
	limit := options.Limit
	if limit == 0 {
		limit = defaultHashScanLimit
	}
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	readID := c.readRequests.allocate()
	defer c.readRequests.complete(readID)
	var resp *pb.HScanResponse
	for attempt := 0; attempt < 2; attempt++ {
		session := c.currentSession()
		resp, err = c.worker.HScan(callCtx, &pb.HScanRequest{
			SessionId:        session.id,
			Key:              &pb.Key{Value: []byte(key)},
			Cursor:           uint64(cursor),
			Limit:            limit,
			ExactHashVersion: hashReadVersionPtr(options.Version),
			ReadRequestId:    readID,
		})
		if err == nil {
			break
		}
		if attempt == 0 && isSessionUnknown(err) {
			if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
				return HashScanResult{}, reopenErr
			}
			continue
		}
		return HashScanResult{}, asDmsError(err)
	}
	entries, err := decodeHashValues(resp.GetEntries())
	if err != nil {
		return HashScanResult{}, err
	}
	return HashScanResult{Version: optionalHashVersion(resp.HashVersion), NextCursor: ScanCursor(resp.NextCursor), Entries: entries}, nil
}

func (c *Client) HWriteAt(ctx context.Context, key, field string, offset uint64, data []byte, options HashRangeWriteOptions) (HashRangeWriteResult, error) {
	if err := validateHashField(field); err != nil {
		return HashRangeWriteResult{}, err
	}
	done, err := c.beginCall(key)
	if err != nil {
		return HashRangeWriteResult{}, err
	}
	defer done()
	durability, err := c.resolveDurability("HashRangeWriteOptions.Durability", options.Durability)
	if err != nil {
		return HashRangeWriteResult{}, err
	}
	callCtx, cancel := boundedContext(ctx, c.timeout)
	defer cancel()
	operationID := c.nextOperation()
	for attempt := 0; attempt < 2; attempt++ {
		session := c.currentSession()
		staged, stageErr := c.stageValue(callCtx, session, data, "go-sdk-hash-range-patch")
		if stageErr != nil {
			if attempt == 0 && isSessionUnknown(stageErr) {
				if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
					return HashRangeWriteResult{}, reopenErr
				}
				continue
			}
			return HashRangeWriteResult{}, asDmsError(stageErr)
		}
		resp, rpcErr := c.worker.HWriteAt(callCtx, &pb.HWriteAtRequest{
			SessionId:           session.id,
			Key:                 &pb.Key{Value: []byte(key)},
			Field:               &pb.HashField{Value: []byte(field)},
			Offset:              offset,
			Value:               staged.value,
			OperationId:         operationID,
			ExpectedHashVersion: hashVersionPtr(options.ExpectedVersion),
			Durability:          string(durability),
		})
		if rpcErr == nil {
			return HashRangeWriteResult{
				HashVersion:  HashVersion(resp.HashVersion),
				ValueVersion: ObjectVersion(resp.ValueVersion),
				Len:          resp.Length,
				FieldCount:   resp.FieldCount,
			}, nil
		}
		cleanupErr := c.cleanupStagedUploads(ctx, session, []stagedUpload{staged}, false)
		if attempt == 0 && isSessionUnknown(rpcErr) {
			cleanupErr = errors.Join(cleanupErr, c.deleteStagedUploads(ctx, session, []stagedUpload{staged}))
			if reopenErr := c.reopenSession(callCtx, session); reopenErr != nil {
				return HashRangeWriteResult{}, setFailureWithCleanup(reopenErr, cleanupErr)
			}
			continue
		}
		return HashRangeWriteResult{}, setFailureWithCleanup(rpcErr, cleanupErr)
	}
	return HashRangeWriteResult{}, asDmsError(protocolError("HWriteAt retry exhausted"))
}

func (c *Client) stageValue(ctx context.Context, session sessionSnapshot, value []byte, purpose string) (stagedUpload, error) {
	alloc, err := c.worker.AllocateStaging(ctx, &pb.AllocateStagingRequest{
		SessionId: session.id,
		Length:    uint64(len(value)),
		Purpose:   purpose,
	})
	if err != nil {
		return stagedUpload{}, asDmsError(err)
	}
	if alloc == nil || alloc.Target == nil {
		return stagedUpload{}, setFailureWithCleanup(protocolError("AllocateStaging response is missing payload target"), c.deleteStagingForSession(ctx, session, alloc.GetStagingId()))
	}
	receipt, err := c.uploadForSession(ctx, session, alloc.Target, value)
	if err != nil {
		return stagedUpload{}, setFailureWithCleanup(err, c.deleteStagingForSession(ctx, session, alloc.StagingId))
	}
	return stagedUpload{
		id:      alloc.StagingId,
		value:   &pb.StagedValue{StagingId: alloc.StagingId, Receipt: receipt},
		receipt: receipt,
	}, nil
}

func (c *Client) cleanupStagedUploads(ctx context.Context, session sessionSnapshot, staged []stagedUpload, deleteStaging bool) error {
	var cleanupErr error
	for _, upload := range staged {
		cleanupErr = errors.Join(cleanupErr, c.releaseWriteFromReceiptForSession(ctx, session, upload.receipt))
		if deleteStaging {
			cleanupErr = errors.Join(cleanupErr, c.deleteStagingForSession(ctx, session, upload.id))
		}
	}
	return cleanupErr
}

func (c *Client) deleteStagedUploads(ctx context.Context, session sessionSnapshot, staged []stagedUpload) error {
	var cleanupErr error
	for _, upload := range staged {
		cleanupErr = errors.Join(cleanupErr, c.deleteStagingForSession(ctx, session, upload.id))
	}
	return cleanupErr
}

func decodeMSetResult(resp *pb.MSetResponse, expected int) (MSetResult, error) {
	if resp == nil || len(resp.Versions) != expected {
		return MSetResult{}, asDmsError(protocolError("MSet response count does not match request"))
	}
	versions := make([]KeyVersion, 0, len(resp.Versions))
	for _, item := range resp.Versions {
		if item == nil || item.Key == nil {
			return MSetResult{}, asDmsError(protocolError("MSet response item is missing key"))
		}
		key := string(item.Key.Value)
		if err := validateKey(key); err != nil {
			return MSetResult{}, asDmsError(protocolError("MSet response contains an invalid key"))
		}
		versions = append(versions, KeyVersion{Key: key, Version: ObjectVersion(item.Version)})
	}
	return MSetResult{Versions: versions}, nil
}

func decodeHashGet(resp *pb.HGetResponse) (HashValue, bool, error) {
	if resp == nil {
		return HashValue{}, false, asDmsError(protocolError("Hash read response is empty"))
	}
	if !resp.Found {
		if resp.Value != nil {
			return HashValue{}, false, asDmsError(protocolError("missing Hash field carries a value"))
		}
		return HashValue{}, false, nil
	}
	if resp.Value == nil {
		return HashValue{}, false, asDmsError(protocolError("Hash hit has no value"))
	}
	value, err := decodeHashValue(resp.Value)
	return value, err == nil, err
}

func decodeHashValues(values []*pb.HashValueRead) ([]HashValue, error) {
	decoded := make([]HashValue, 0, len(values))
	for _, value := range values {
		item, err := decodeHashValue(value)
		if err != nil {
			return nil, err
		}
		decoded = append(decoded, item)
	}
	return decoded, nil
}

func decodeHashValue(value *pb.HashValueRead) (HashValue, error) {
	if value == nil || value.Field == nil {
		return HashValue{}, asDmsError(protocolError("Hash value is missing field"))
	}
	field := string(value.Field.Value)
	if err := validateHashField(field); err != nil {
		return HashValue{}, asDmsError(protocolError("Hash response contains an invalid field"))
	}
	if len(value.Segments) != 0 {
		return HashValue{}, unimplemented("segmented Hash payload is not implemented by the Go SDK")
	}
	if uint64(len(value.InlineValue)) != value.LogicalLength {
		return HashValue{}, asDmsError(protocolError("Hash inline value length does not match metadata"))
	}
	return HashValue{
		Field:        field,
		HashVersion:  HashVersion(value.HashVersion),
		ValueVersion: ObjectVersion(value.ValueVersion),
		Bytes:        value.InlineValue,
	}, nil
}

func objectVersionPtr(value *ObjectVersion) *uint64 {
	if value == nil {
		return nil
	}
	result := uint64(*value)
	return &result
}

func hashVersionPtr(value *HashVersion) *uint64 {
	if value == nil {
		return nil
	}
	result := uint64(*value)
	return &result
}

func hashReadVersionPtr(value HashReadVersion) *uint64 {
	return hashVersionPtr(value.exact)
}

func optionalHashVersion(value *uint64) *HashVersion {
	if value == nil {
		return nil
	}
	result := HashVersion(*value)
	return &result
}
