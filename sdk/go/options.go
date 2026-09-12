package dms

import (
	"os"
	"strconv"
	"strings"
	"time"
)

const (
	defaultTimeout           = 30 * time.Second
	defaultHeartbeatInterval = 10 * time.Second
	defaultInlineThreshold   = 64 * 1024
	defaultSessionCapacity   = 64
	defaultMaxMessageBytes   = 16 * 1024 * 1024
	maxDurationMillis        = uint64(1<<63-1) / uint64(time.Millisecond)
	maxSessionCapacity       = 1 << 20
)

func resolveOptions(explicitEndpoint string, options ClientOptions) (resolvedOptions, error) {
	resolved := resolvedOptions{
		timeout:                defaultTimeout,
		defaultDurability:      DurabilityLocalMemory,
		inlineThresholdBytes:   defaultInlineThreshold,
		heartbeatInterval:      defaultHeartbeatInterval,
		sessionChannelCapacity: defaultSessionCapacity,
		tls:                    TLSDisabled,
	}
	if value := os.Getenv("DMS_ENDPOINT"); value != "" {
		resolved.endpoint = value
	}
	if value := os.Getenv("DMS_TIMEOUT_MILLIS"); value != "" {
		duration, err := parseMillis("DMS_TIMEOUT_MILLIS", value)
		if err != nil {
			return resolvedOptions{}, err
		}
		resolved.timeout = duration
	}
	if value := os.Getenv("DMS_DEFAULT_DURABILITY"); value != "" {
		durability, err := parseDurability("DMS_DEFAULT_DURABILITY", value)
		if err != nil {
			return resolvedOptions{}, err
		}
		resolved.defaultDurability = durability
	}
	if value := os.Getenv("DMS_INLINE_THRESHOLD_BYTES"); value != "" {
		threshold, err := parseUint64("DMS_INLINE_THRESHOLD_BYTES", value)
		if err != nil {
			return resolvedOptions{}, err
		}
		resolved.inlineThresholdBytes = threshold
	}
	if value := os.Getenv("DMS_HEARTBEAT_INTERVAL_MILLIS"); value != "" {
		interval, err := parseMillis("DMS_HEARTBEAT_INTERVAL_MILLIS", value)
		if err != nil {
			return resolvedOptions{}, err
		}
		resolved.heartbeatInterval = interval
	}
	if value := os.Getenv("DMS_SESSION_CHANNEL_CAPACITY"); value != "" {
		// 兼容跨语言配置面：Go SDK 当前没有本地 Session 消息队列，
		// 这里只验证并保留配置值，不把它传给内部 heartbeat loop。
		capacity, err := parseSessionCapacity("DMS_SESSION_CHANNEL_CAPACITY", value)
		if err != nil {
			return resolvedOptions{}, err
		}
		resolved.sessionChannelCapacity = capacity
	}
	if value := os.Getenv("DMS_SHARED_MEMORY"); value != "" {
		shared, err := parseBool("DMS_SHARED_MEMORY", value)
		if err != nil {
			return resolvedOptions{}, err
		}
		resolved.sharedMemory = shared
	}
	if value := os.Getenv("DMS_TLS_MODE"); value != "" {
		tls, err := parseTLS("DMS_TLS_MODE", value)
		if err != nil {
			return resolvedOptions{}, err
		}
		resolved.tls = tls
	}

	if options.Endpoint != "" {
		resolved.endpoint = options.Endpoint
	}
	if explicitEndpoint != "" {
		resolved.endpoint = explicitEndpoint
	}
	if options.Timeout != 0 {
		if options.Timeout < 0 {
			return resolvedOptions{}, invalidArgument("Timeout must be positive")
		}
		resolved.timeout = options.Timeout
	}
	if options.DefaultDurability != "" {
		durability, err := parseDurability("DefaultDurability", string(options.DefaultDurability))
		if err != nil {
			return resolvedOptions{}, err
		}
		resolved.defaultDurability = durability
	}
	if options.InlineThresholdBytes != nil {
		resolved.inlineThresholdBytes = *options.InlineThresholdBytes
	}
	if options.HeartbeatInterval != 0 {
		if options.HeartbeatInterval < 0 {
			return resolvedOptions{}, invalidArgument("HeartbeatInterval must be positive")
		}
		resolved.heartbeatInterval = options.HeartbeatInterval
	}
	if options.SessionChannelCapacity != nil {
		if err := validateSessionCapacity("SessionChannelCapacity", *options.SessionChannelCapacity); err != nil {
			return resolvedOptions{}, err
		}
		resolved.sessionChannelCapacity = *options.SessionChannelCapacity
	}
	if options.SharedMemory != nil {
		resolved.sharedMemory = *options.SharedMemory
	}
	if options.TLS != "" {
		tls, err := parseTLS("TLS", string(options.TLS))
		if err != nil {
			return resolvedOptions{}, err
		}
		resolved.tls = tls
	}
	if resolved.endpoint == "" {
		return resolvedOptions{}, invalidArgument("DMS endpoint is required")
	}
	if resolved.tls != TLSDisabled {
		return resolvedOptions{}, invalidArgument("only disabled TLS is supported by the Go SDK")
	}
	return resolved, nil
}

func parseMillis(name, value string) (time.Duration, error) {
	millis, err := parsePositiveUint64(name, value)
	if err != nil {
		return 0, err
	}
	if millis > maxDurationMillis {
		return 0, invalidArgument(name + " is too large")
	}
	return time.Duration(millis) * time.Millisecond, nil
}

func parseUint64(name, value string) (uint64, error) {
	parsed, err := strconv.ParseUint(value, 10, 64)
	if err != nil {
		return 0, invalidArgument(name + " must be an unsigned integer")
	}
	return parsed, nil
}

func parsePositiveUint64(name, value string) (uint64, error) {
	parsed, err := parseUint64(name, value)
	if err != nil {
		return 0, err
	}
	if parsed == 0 {
		return 0, invalidArgument(name + " must be a positive unsigned integer")
	}
	return parsed, nil
}

func parseSessionCapacity(name, value string) (uint64, error) {
	parsed, err := parsePositiveUint64(name, value)
	if err != nil {
		return 0, err
	}
	if err := validateSessionCapacity(name, parsed); err != nil {
		return 0, err
	}
	return parsed, nil
}

func validateSessionCapacity(name string, capacity uint64) error {
	if capacity == 0 {
		return invalidArgument(name + " must be positive")
	}
	if capacity > maxSessionCapacity {
		return invalidArgument(name + " exceeds the SDK limit")
	}
	return nil
}

func parseDurability(name, value string) (DurabilityPolicy, error) {
	switch DurabilityPolicy(value) {
	case DurabilityLocalMemory:
		return DurabilityLocalMemory, nil
	default:
		return "", invalidArgument(name + " must be local-memory")
	}
}

func parseTLS(name, value string) (ClientTLSOptions, error) {
	switch ClientTLSOptions(value) {
	case TLSDisabled:
		return TLSDisabled, nil
	default:
		return "", invalidArgument(name + " must be disabled")
	}
}

func parseBool(name, value string) (bool, error) {
	switch strings.ToLower(value) {
	case "1", "true", "yes", "on":
		return true, nil
	case "0", "false", "no", "off":
		return false, nil
	default:
		return false, invalidArgument(name + " must be a boolean")
	}
}

func formatUint64(value uint64) string {
	return strconv.FormatUint(value, 10)
}
