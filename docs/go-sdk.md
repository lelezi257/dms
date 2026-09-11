# Go SDK

当前是可从公开源码取得的开发预览 SDK，未发布正式语义版本：纯 Go，通过 gRPC 连接 Node；Linux 同机还可使用 UDS 和共享内存，不需要 Rust native extension 或 CGO。用户导入的是 `dms` 包，不是 protobuf 生成类型。

本阶段提供对象子集 **Set、Get、Del、Stat、Scan**，以及条件写、指定版本/范围读的 Options；本地候选还增加 **SetFrom、GetInto、GetReader**，使用这些新接口应按下文构建候选包，不能假定旧公开伪版本已经包含。不表示已覆盖 Rust SDK 的全部方法，也不承诺 Go SDK 已具备 TLS、跨语言观测适配等能力。

## 候选新增：用户内存与 Reader

| 方法 | 使用场景 | 数据/生命周期 |
| --- | --- | --- |
| `GetInto(ctx, key, dst, options)` | 用户已准备 `[]byte` | 返回 `GetIntoResult{Version, Len}, found, err`；容量不足报错且不写越界 |
| `GetReader(ctx, key, options)` | 用户稍后才通过 `Read(dst)` 提供内存 | 返回 `ReadResult{Version, Len, Body}, found, err`；一次 Get 固定版本，不因每次 Read 再查 Current |
| `SetFrom(ctx, key, src, length, options)` | 已有 `io.Reader`，避免调用方先 ReadAll | 精确消费 length，短源失败不发布，超过 length 的部分不消费；返回原 `SetResult` |

`Get/Set` 保留：Get 返回 owned bytes；Set 已接收用户 bytes，所以没有额外的 SetInto。

```go
result, found, err := client.GetReader(ctx, "demo/key", dms.GetOptions{})
if err != nil { return err }
if !found { return fmt.Errorf("对象不存在") }
defer result.Body.Close() // 提前退出也释放；读到 EOF 会自动释放本次保护
buffer := make([]byte, 4096)
for {
    n, err := result.Body.Read(buffer)
    // 先消费 buffer[:n]，再处理 err；最后一次可能同时返回数据与 EOF。
    consume(buffer[:n])
    if err == io.EOF { break }
    if err != nil { return err }
}
```

上例是业务函数片段，`consume` 由应用实现。传入的 ctx/default timeout 也覆盖后续 Reader 消费；不要无限保留未读完的 Body。Client.Close 会取消并关闭尚存 Reader 后才释放 mmap。

范围仍默认严格。显式 `GetOptions{Range: &dms.ByteRange{Offset: off, Len: n}, ClampRange: true}` 才裁剪尾部；offset 到/超过 EOF 返回空结果，溢出仍报错，Len=0 表示空范围。Node 在同一已解析版本上处理，不需要调用者先 Stat。

SHM 的 Read/GetInto 从共享映射复制到 dst，SetFrom 直接写 staging；TCP 仍有 protobuf payload 缓冲，不能把 Reader 理解成网络零拷贝或一个新双向流协议。Scan 可指定 Delimiter；`ObjectInfo.IsPrefix` 区分合成目录前缀与真实对象，游标绑定 prefix/delimiter，分页不承诺全局快照。

## 1. 安装固定源码版本

在Linux消费者项目中，使用Go 1.25或兼容工具链；不需要DMS源码目录、Rust或protoc。

```bash
mkdir dms-demo
cd dms-demo
go mod init example.com/dms-demo
GOPROXY=https://proxy.golang.org,direct go get github.com/lelezi257/dms/sdk/go@v0.0.0-20260910013452-f4555eac2319
```

这个Go伪版本固定到DMS提交 `f4555eac23190ceef555b284366e4623c48fb72b`，不是正式v0.1.0 Release。配套JuiceFS与服务代码见[接入指南](juicefs.md)。无需设置GONOSUMDB；公开下载保留Go校验和验证。以下第2节程序可以直接放入本目录。

### 开发者测试尚未提交的SDK

只有需要验证本地改动时才用下面的候选proxy，不是上述公开版本的安装前提。

先按[单 VM 手册](local-single-vm-manual.md)进入 Linux，在源码根准备环境。需要 Go 1.25 或兼容工具链，消费者不需要 protoc。

```bash
source scripts/env.sh
export REPO="$PWD"
export GO_PROXY="$REPO/.local/go-proxy"
go version
python3 scripts/sdk/package_go.py --output "$GO_PROXY"
```

该命令生成符合 Go module proxy 格式的候选包，输出 `version`、`proxy` 和归档 `sha256`，不向公网发布。将输出中的版本填入下面变量；不要把占位符原样执行：

```bash
export SDK_VERSION='<上一步输出的 v0.1.0-dev.… 版本>'
export GOPROXY="file://$GO_PROXY,https://proxy.golang.org"
export GONOSUMDB='github.com/lelezi257/dms/sdk/go'
export GOWORK=off
mkdir -p "$REPO/.local/go-demo"
cd "$REPO/.local/go-demo"
go mod init example.com/dms-demo
go get "github.com/lelezi257/dms/sdk/go@$SDK_VERSION"
```

这里 `go get` 从本地候选仓下载未提交的 SDK，公网代理只用于其它依赖；不是宣称该候选版本已经公开发布。候选包来源和 SHA256 应由交付方核对。独立消费者可以拿到整个 proxy 目录后安装，不需要项目源码或 `replace`。不要把实验proxy设置带入公开版本的独立获取验收。

## 2. 最小读写程序

先启动真实 Node/Meta，见[单 VM 手册](local-single-vm-manual.md)或 [JuiceFS 接入](juicefs.md)。将下面保存为消费者目录的 `main.go`，endpoint 使用实际 Node 地址：

```go
package main

import (
    "context"
    "errors"
    "fmt"
    "os"
    "time"

    dms "github.com/lelezi257/dms/sdk/go"
)

func run() error {
    ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
    defer cancel()
    client, err := dms.Connect(ctx, os.Getenv("DMS_ENDPOINT"), dms.ClientOptions{})
    if err != nil {
        return err
    }
    defer client.Close()

    written, err := client.Set(ctx, "demo/key", []byte("hello"))
    if err != nil {
        return err
    }
    value, found, err := client.Get(ctx, "demo/key")
    if err != nil {
        return err
    }
    if !found {
        return fmt.Errorf("刚写入的对象缺失")
    }
    fmt.Printf("version=%d value=%q\n", written.Version, value)
    deleted, err := client.Del(ctx, "demo/key")
    if err != nil {
        return err
    }
    fmt.Printf("deleted=%t\n", deleted.Deleted)
    return nil
}

func main() {
    if err := run(); err != nil {
        var de *dms.DmsError
        if errors.As(err, &de) {
            fmt.Fprintf(os.Stderr, "code=0x%08x kind=%s message=%s\n",
                uint32(de.Code), de.Kind, de.Message)
        } else {
            fmt.Fprintln(os.Stderr, err)
        }
        os.Exit(1)
    }
}
```

```bash
export DMS_ENDPOINT=http://127.0.0.1:25200
CGO_ENABLED=0 go run .
```

`ctx` 控制本次连接/请求的等待，不是 Client 的生命周期开关。长期复用 Client，退出时 `Close()`：停止心跳、释放连接和 mmap；已开始的调用仍需结束，不应每个 Set 都重新连接。

## 3. 返回值先看什么

| API | 成功时返回 | 缺失与失败 |
| --- | --- | --- |
| `Set(ctx, key, value)` | `SetResult{Version, Len}` | 失败返回 `error`；空 value 是存在的零长度对象 |
| `Get(ctx, key)` | `([]byte, true, nil)` | 缺失为 `(nil, false, nil)`；网络/超时等返回非 nil error |
| `GetWithOptions(ctx, key, options)` | `GetResult{Version, Bytes}, true, nil` | 同 Get；可固定版本和读取范围 |
| `Del(ctx, key)` | `DeleteResult{Deleted, Version}` | 已缺失时 `Deleted=false, err=nil`，不是网络失败 |
| `Stat(ctx, key)` | `ObjectInfo{Key, Len, ModifiedTime, Version}, true, nil` | 缺失 `found=false`；读取属性不下载 value |
| `Scan(ctx, prefix, options)` | `ScanResult{Items, NextCursor}` | 下一页游标为空才表示结束；错误不能当结束 |

这与 Rust 的结果语义对应：`Ok(Some(value))` → `found=true`；`Ok(None)` → `found=false`；`Err(error)` → 非 nil error。必须先判断 `err`，再判断 `found`，不能通过 message 匹配“找不到”。`Stat.ModifiedTime` 来自对象元数据，反复 Stat 不会生成新的修改时间。

`DmsError.Code` 是保留未知值能力的 `uint32` 数字身份，程序比较导出的错误码常量；`Kind` 是通用类别，`Message` 用于诊断。`errors.As` 能从连接错误等包装中取出它。一次写超时不证明没有提交，不能无条件换一个新请求重写或删除可能已提交的数据。

### 范围读与条件写

```go
result, err := client.SetWithOptions(ctx, "demo/key", []byte("abcdef"),
    dms.SetOptions{Condition: dms.WriteIfAbsent()})
if err != nil { return err }

part, found, err := client.GetWithOptions(ctx, "demo/key", dms.GetOptions{
    Version: dms.ReadExact(result.Version),
    Range: &dms.ByteRange{Offset: 2, Len: 3}, // [2,5)，得到 "cde"
})
_ = part
_ = found
if err != nil { return err }
```

`Range=nil` 读完整对象；`Len=0` 是零长度，不是“读到结尾”。条件写由 Node/Meta 执行，不用客户端 Get+Set 模拟 CAS。

### 分页列举

```go
options := dms.ScanOptions{Limit: 1000}
for {
    page, err := client.Scan(ctx, "demo/", options)
    if err != nil { return err }
    for _, item := range page.Items {
        fmt.Println(item.Key, item.Len, item.ModifiedTime)
    }
    if page.NextCursor == "" { break }
    options = dms.ScanOptions{Cursor: page.NextCursor}
}
```

游标不解析、不自行拼接；后续页保持相同 prefix。`StartAfter` 只用于第一页，不能与 Cursor 同时设置。静态集合分页和并发修改下的快照是不同保证，不把当前接口解释为全局事务快照。

## 4. TCP 与同机 SHM

普通调用不变，分支只在连接配置和内部传输实现：

```bash
# TCP：可以连接远端 Node。
export DMS_ENDPOINT=http://127.0.0.1:25200
export DMS_SHARED_MEMORY=false

# 或者同机 UDS + SHM；必须是 Node 实际监听的路径。
export DMS_ENDPOINT="unix://$REPO/.local/juicefs-demo/worker.sock"
export DMS_SHARED_MEMORY=true
```

SHM 需要 Linux、同机可信进程及 socket 访问权限。SDK 内部通过 FD 通道取得 Region 并复用 mmap；用户不管理 FD、偏移或共享内存生命周期。小 Set 可能直接内联在 RPC 中，不是启用 SHM 就每条请求都必须走共享页。

普通 Get 在 TCP/SHM 下均返回调用者拥有的 `[]byte`；SHM 路径会复制出结果后归还共享读保护，不是公开的零拷贝 View API。SDK 不跨请求缓存 value，只复用连接和 Region 映射。Set 返回前不要并发修改传入的 slice。

配置优先级为 **默认值 < 环境变量 < 显式 API**；`Connect` 非空 endpoint 参数优先于 Options.Endpoint。常用默认值：请求超时 30s、心跳 10s、内联阈值 64KiB、SharedMemory=false。环境变量包括 `DMS_TIMEOUT_MILLIS`、`DMS_HEARTBEAT_INTERVAL_MILLIS`、`DMS_INLINE_THRESHOLD_BYTES`；完整字段见 [types.go](../sdk/go/types.go)。没有隐式默认 endpoint。

本阶段 Go SDK 仅支持 `local-memory` 和明文 gRPC，TLS 配置会明确拒绝，不能用于不可信公网。写入成功代表当前内存策略下提交成功，不代表掉电后恢复。

## 5. 阅读源码

- [client.go](../sdk/go/client.go)：公开 API、会话与关闭。
- [types.go](../sdk/go/types.go)：原生参数/返回结构。
- [errors.go](../sdk/go/errors.go)：数字错误码、Kind 与错误包装。
- [候选构包脚本](../scripts/sdk/package_go.py)：无需本地 replace 的 module proxy。

功能/故障/性能验证分别记录；本页是使用合同，不是 B/C 阶段验收结论。
