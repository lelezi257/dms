package dms

import (
	"io"
	"time"
)

// ObjectVersion 对齐 Rust SDK 的 ObjectVersion。0 只作为 wire/零值存在，
// 正常已提交对象版本由 Meta 分配。
type ObjectVersion uint64

// DurabilityPolicy 当前只支持 LocalMemory；其它值保留为未来能力，显式传入会报错。
type DurabilityPolicy string

const (
	DurabilityLocalMemory DurabilityPolicy = "local-memory"
)

// ClientTLSOptions 预留 TLS 模式；当前 Go SDK 只实现明文/insecure gRPC。
type ClientTLSOptions string

const (
	TLSDisabled ClientTLSOptions = "disabled"
)

// ClientOptions 采用 默认 < 环境变量 < 显式 API 的解析顺序。指针字段用于区分
// “未设置”和“显式设置为 false/0”；Go 新 SDK 不提供旧 value cache 开关。
type ClientOptions struct {
	Endpoint             string
	Timeout              time.Duration
	DefaultDurability    DurabilityPolicy
	InlineThresholdBytes *uint64
	HeartbeatInterval    time.Duration
	// SessionChannelCapacity 是与其它 SDK 对齐的兼容配置。当前 Go SDK 的 Session
	// 通道只发送 heartbeat/ack，没有本地业务消息队列，因此该值只做解析校验，
	// 不参与内部 goroutine/channel 分配；后续若引入本地队列，必须先补公开语义说明。
	SessionChannelCapacity *uint64
	SharedMemory           *bool
	TLS                    ClientTLSOptions
}

type resolvedOptions struct {
	endpoint               string
	timeout                time.Duration
	defaultDurability      DurabilityPolicy
	inlineThresholdBytes   uint64
	heartbeatInterval      time.Duration
	sessionChannelCapacity uint64
	sharedMemory           bool
	tls                    ClientTLSOptions
}

// SetOptions 保留服务端条件写和本次可靠性选择；条件由服务端执行，SDK 不用 Get+Set
// 自己模拟 CAS。
type SetOptions struct {
	Condition  WriteCondition
	Durability DurabilityPolicy
}

// SetResult 保留版本与长度，供上层做缓存/索引/审计；即使 JuiceFS adapter 暂时忽略，
// SDK 也不能丢字段。
type SetResult struct {
	Version ObjectVersion
	Len     uint64
}

// GetIntoResult 是 GetInto 的命中返回；Len 是本次选中范围长度。
type GetIntoResult struct {
	Version ObjectVersion
	Len     uint64
}

// DeleteResult 区分“本次确实发布 tombstone”和“此前已经缺失”。
type DeleteResult struct {
	Deleted bool
	Version ObjectVersion
}

// GetResult 是 GetWithOptions 的命中返回；Bytes 是调用者 owned 内存。
type GetResult struct {
	Version ObjectVersion
	Bytes   []byte
}

// ReadResult 是 GetReader 的命中返回。Body 必须由调用方关闭；读到 EOF、Close、
// 错误或请求上下文取消都会归还本次固定版本读保护。
type ReadResult struct {
	Version ObjectVersion
	Len     uint64
	Body    io.ReadCloser
}

// ByteRange 表示 [Offset, Offset+Len)。Len==0 是零长度范围，不表示读到尾。
type ByteRange struct {
	Offset uint64
	Len    uint64
}

// GetOptions 保留 Current/Exact 版本选择与可选 Range。Range==nil 表示完整对象。
// ClampRange 默认 false，保持严格范围语义；显式 true 时由 Node 在同一已解析版本
// 上裁剪越过尾部的范围，SDK 不先 Stat。
type GetOptions struct {
	Version    ReadVersion
	Range      *ByteRange
	ClampRange bool
}

// ObjectInfo 是 Stat/Scan 的公共对象属性投影，不暴露 protobuf。
type ObjectInfo struct {
	Key          string
	Len          uint64
	ModifiedTime time.Time
	Version      ObjectVersion
	IsPrefix     bool
}

// ScanOptions.StartAfter 只用于第一页；Cursor 非空时必须只传 Cursor。
type ScanOptions struct {
	Limit      uint32
	StartAfter *string
	Cursor     string
	Delimiter  string
}

type ScanResult struct {
	Items      []ObjectInfo
	NextCursor string
}

type ReadVersion struct {
	exact *ObjectVersion
}

func ReadCurrent() ReadVersion {
	return ReadVersion{}
}

func ReadExact(version ObjectVersion) ReadVersion {
	return ReadVersion{exact: &version}
}

type WriteCondition struct {
	kind    writeConditionKind
	version ObjectVersion
}

type writeConditionKind uint8

const (
	writeAny writeConditionKind = iota
	writeIfAbsent
	writeIfPresent
	writeIfVersion
)

func WriteAny() WriteCondition {
	return WriteCondition{kind: writeAny}
}

func WriteIfAbsent() WriteCondition {
	return WriteCondition{kind: writeIfAbsent}
}

func WriteIfPresent() WriteCondition {
	return WriteCondition{kind: writeIfPresent}
}

func WriteIfVersion(version ObjectVersion) WriteCondition {
	return WriteCondition{kind: writeIfVersion, version: version}
}

func (c WriteCondition) wire() string {
	switch c.kind {
	case writeIfAbsent:
		return "if-absent"
	case writeIfPresent:
		return "if-present"
	case writeIfVersion:
		return "if-version:" + formatUint64(uint64(c.version))
	default:
		return ""
	}
}
