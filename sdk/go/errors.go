package dms

import (
	"context"
	"errors"
	"fmt"

	pb "github.com/lelezi257/dms/sdk/go/internal/pb/dms/v1"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// ErrorCode 是跨语言稳定的 DMS 数字错误码。程序分支应优先匹配 Code/Kind，
// message 只用于诊断显示。
type ErrorCode uint32

// ErrorKind 是语言原生错误分类，不要求调用方依赖 protobuf enum。
type ErrorKind string

const (
	ErrorKindUnknown            ErrorKind = "Unknown"
	ErrorKindInvalidArgument    ErrorKind = "InvalidArgument"
	ErrorKindNotFound           ErrorKind = "NotFound"
	ErrorKindAlreadyExists      ErrorKind = "AlreadyExists"
	ErrorKindPermissionDenied   ErrorKind = "PermissionDenied"
	ErrorKindResourceExhausted  ErrorKind = "ResourceExhausted"
	ErrorKindFailedPrecondition ErrorKind = "FailedPrecondition"
	ErrorKindAborted            ErrorKind = "Aborted"
	ErrorKindUnimplemented      ErrorKind = "Unimplemented"
	ErrorKindInternal           ErrorKind = "Internal"
	ErrorKindUnavailable        ErrorKind = "Unavailable"
	ErrorKindDataLoss           ErrorKind = "DataLoss"
	ErrorKindUnauthenticated    ErrorKind = "Unauthenticated"
	ErrorKindDeadlineExceeded   ErrorKind = "DeadlineExceeded"
)

const (
	CLIENT_ARGUMENT_INVALID              ErrorCode = 0x01010001
	CLIENT_CONNECTION_UNAVAILABLE        ErrorCode = 0x01020001
	CLIENT_DEADLINE_EXCEEDED             ErrorCode = 0x01020002
	CLIENT_PROTOCOL_INVALID_ERROR_DETAIL ErrorCode = 0x01030001
	CLIENT_PROTOCOL_VIOLATION            ErrorCode = 0x01030002
	CLIENT_PERMISSION_DENIED             ErrorCode = 0x01040001
	NODE_TRANSFER_UNSUPPORTED            ErrorCode = 0x02040001
)

// DmsError 是 Go SDK 的原生错误载体。服务端 ErrorDetail 可在协议边界还原为
// 这个类型；未知 gRPC 状态也会映射为同一个结构而不是让调用方解析字符串。
type DmsError struct {
	Code    ErrorCode
	Kind    ErrorKind
	Message string
	Cause   error
}

func (e *DmsError) Error() string {
	if e == nil {
		return "<nil>"
	}
	return fmt.Sprintf("dms error code=%d kind=%s: %s", e.Code, e.Kind, e.Message)
}

func (e *DmsError) Unwrap() error {
	if e == nil {
		return nil
	}
	return e.Cause
}

func newDmsError(code ErrorCode, kind ErrorKind, message string) *DmsError {
	return &DmsError{Code: code, Kind: kind, Message: message}
}

func wrapDmsError(code ErrorCode, kind ErrorKind, message string, cause error) *DmsError {
	return &DmsError{Code: code, Kind: kind, Message: message, Cause: cause}
}

func invalidArgument(message string) *DmsError {
	return newDmsError(CLIENT_ARGUMENT_INVALID, ErrorKindInvalidArgument, message)
}

func protocolError(message string) *DmsError {
	return newDmsError(CLIENT_PROTOCOL_VIOLATION, ErrorKindInternal, message)
}

func unimplemented(message string) *DmsError {
	return newDmsError(NODE_TRANSFER_UNSUPPORTED, ErrorKindUnimplemented, message)
}

// ConnectError 与 Rust SDK 的 ConnectError 对齐；errors.As 可取出内部 DmsError。
type ConnectError struct {
	Err *DmsError
}

func (e *ConnectError) Error() string {
	if e == nil || e.Err == nil {
		return "failed to connect DMS client"
	}
	return "failed to connect DMS client: " + e.Err.Error()
}

func (e *ConnectError) Unwrap() error {
	if e == nil {
		return nil
	}
	return e.Err
}

func asDmsError(err error) error {
	if err == nil {
		return nil
	}
	var dmsErr *DmsError
	if errors.As(err, &dmsErr) {
		return dmsErr
	}
	if errors.Is(err, context.Canceled) {
		return wrapDmsError(CLIENT_DEADLINE_EXCEEDED, ErrorKindDeadlineExceeded, err.Error(), err)
	}
	st, ok := status.FromError(err)
	if !ok {
		return wrapDmsError(CLIENT_CONNECTION_UNAVAILABLE, ErrorKindUnavailable, err.Error(), err)
	}
	for _, detail := range st.Details() {
		if wire, ok := detail.(*pb.ErrorDetail); ok {
			return &DmsError{
				Code:    ErrorCode(wire.DmsCode),
				Kind:    nativeKind(wire.Kind),
				Message: wire.Message,
				Cause:   err,
			}
		}
	}
	if st.Code() == codes.DeadlineExceeded || st.Code() == codes.Canceled {
		return wrapDmsError(CLIENT_DEADLINE_EXCEEDED, ErrorKindDeadlineExceeded, st.Message(), err)
	}
	return wrapDmsError(CLIENT_CONNECTION_UNAVAILABLE, grpcKind(st.Code()), st.Message(), err)
}

func nativeKind(kind pb.ErrorKind) ErrorKind {
	switch kind {
	case pb.ErrorKind_ERROR_KIND_INVALID_ARGUMENT:
		return ErrorKindInvalidArgument
	case pb.ErrorKind_ERROR_KIND_NOT_FOUND:
		return ErrorKindNotFound
	case pb.ErrorKind_ERROR_KIND_ALREADY_EXISTS:
		return ErrorKindAlreadyExists
	case pb.ErrorKind_ERROR_KIND_PERMISSION_DENIED:
		return ErrorKindPermissionDenied
	case pb.ErrorKind_ERROR_KIND_RESOURCE_EXHAUSTED:
		return ErrorKindResourceExhausted
	case pb.ErrorKind_ERROR_KIND_FAILED_PRECONDITION:
		return ErrorKindFailedPrecondition
	case pb.ErrorKind_ERROR_KIND_ABORTED:
		return ErrorKindAborted
	case pb.ErrorKind_ERROR_KIND_UNIMPLEMENTED:
		return ErrorKindUnimplemented
	case pb.ErrorKind_ERROR_KIND_INTERNAL:
		return ErrorKindInternal
	case pb.ErrorKind_ERROR_KIND_UNAVAILABLE:
		return ErrorKindUnavailable
	case pb.ErrorKind_ERROR_KIND_DATA_LOSS:
		return ErrorKindDataLoss
	case pb.ErrorKind_ERROR_KIND_UNAUTHENTICATED:
		return ErrorKindUnauthenticated
	case pb.ErrorKind_ERROR_KIND_DEADLINE_EXCEEDED:
		return ErrorKindDeadlineExceeded
	default:
		return ErrorKindUnknown
	}
}

func grpcKind(code codes.Code) ErrorKind {
	switch code {
	case codes.InvalidArgument:
		return ErrorKindInvalidArgument
	case codes.NotFound:
		return ErrorKindNotFound
	case codes.AlreadyExists:
		return ErrorKindAlreadyExists
	case codes.PermissionDenied:
		return ErrorKindPermissionDenied
	case codes.ResourceExhausted:
		return ErrorKindResourceExhausted
	case codes.FailedPrecondition:
		return ErrorKindFailedPrecondition
	case codes.Aborted:
		return ErrorKindAborted
	case codes.Unimplemented:
		return ErrorKindUnimplemented
	case codes.Internal:
		return ErrorKindInternal
	case codes.Unavailable:
		return ErrorKindUnavailable
	case codes.DataLoss:
		return ErrorKindDataLoss
	case codes.Unauthenticated:
		return ErrorKindUnauthenticated
	case codes.DeadlineExceeded:
		return ErrorKindDeadlineExceeded
	case codes.Canceled:
		return ErrorKindDeadlineExceeded
	default:
		return ErrorKindUnknown
	}
}
