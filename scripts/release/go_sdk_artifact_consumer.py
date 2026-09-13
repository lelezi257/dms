#!/usr/bin/env python3
"""验证 Go SDK 候选包可以被外部项目按发布件方式使用。

这个脚本模拟真实用户：

1. 只拿到一个 file:// Go module proxy 和一个版本号；
2. 在临时目录生成自己的 go.mod；
3. 禁止 replace/path 依赖；
4. 连接已经启动的 DMS Worker；
5. 用公开 Go SDK API 覆盖首批试用需要的对象、Hash/KKV、range、批量、
   分页、Reader/buffer、错误分类能力。

脚本不启动/停止 dms-node 或 dms-meta，调用方需要通过 --endpoint 传入已经
启动好的 Worker endpoint。
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap

MODULE = "github.com/lelezi257/dms/sdk/go"


def run(args, cwd: Path, env: dict[str, str], log: Path) -> None:
    """运行命令并把 stdout/stderr 同时写入验收日志。"""
    line = "$ " + " ".join(map(str, args)) + "\n"
    with log.open("a", encoding="utf-8") as out:
        out.write(line)
        out.flush()
        subprocess.run(args, cwd=cwd, env=env, check=True, stdout=out, stderr=subprocess.STDOUT)


def latest_version(proxy: Path, module: str = MODULE) -> str:
    """从 file:// module proxy 的 @v/list 读取唯一或最新候选版本。"""
    list_file = proxy / module / "@v/list"
    versions = [line.strip() for line in list_file.read_text(encoding="utf-8").splitlines() if line.strip()]
    if not versions:
        raise RuntimeError(f"no Go SDK versions found in {list_file}")
    return versions[-1]


def write_consumer(directory: Path, version: str) -> None:
    """生成外部用户项目；这里故意不写 replace，防止误用本地源码。"""
    (directory / "go.mod").write_text(
        textwrap.dedent(
            f"""
            module dms-go-sdk-artifact-consumer

            go 1.23

            require {MODULE} {version}
            """
        ).lstrip(),
        encoding="utf-8",
    )
    (directory / "main.go").write_text(CONSUMER_GO, encoding="utf-8")


def assert_no_local_dependency(directory: Path) -> None:
    go_mod = (directory / "go.mod").read_text(encoding="utf-8")
    # module path 本身包含 /sdk/go；这里检查的是用户项目是否绕过 module proxy，
    # 例如 replace 到相对路径、绝对源码路径或 file URL。
    forbidden = ("replace ", " => ", "file://", "../", "/source/")
    for token in forbidden:
        if token in go_mod:
            raise RuntimeError(f"go.mod contains local dependency marker {token!r}")


def consumer_env(base_env: dict[str, str], proxy: Path, endpoint: str, shared_memory: bool, work_dir: Path) -> dict[str, str]:
    """构造隔离消费者环境；Go 的构建缓存和模块缓存必须落在本次临时目录内。"""
    env = dict(base_env)
    # DMS SDK 必须从 file:// 候选 proxy 下载；第三方依赖允许走标准 Go proxy，
    # 避免把 DMS 私有源码路径当成依赖缓存的一部分。
    env.update(
        GOPROXY=proxy.as_uri() + ",https://proxy.golang.org,direct",
        # 只有尚未进入公共 checksum database 的本地 DMS 候选包需要跳过
        # sumdb。第三方模块继续使用 Go 默认的 checksum 校验，避免发布验收
        # 因为一个私有候选依赖而关闭整个依赖图的供应链完整性保护。
        GONOSUMDB=MODULE,
        DMS_ENDPOINT=endpoint,
        DMS_SHARED_MEMORY="true" if shared_memory else "false",
        DMS_TIMEOUT_MILLIS="5000",
        GOCACHE=str(work_dir / "gocache"),
        GOMODCACHE=str(work_dir / "gomodcache"),
    )
    # 发行验证 VM 可能把 Go 安装在 /usr/local/go/bin，但没有写入登录 shell PATH。
    # 这里只影响临时消费者进程，不修改用户环境。
    if Path("/usr/local/go/bin/go").exists():
        env["PATH"] = "/usr/local/go/bin:" + env.get("PATH", "")
    return env


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--proxy", type=Path, required=True, help="package_go.py 生成的 Go module proxy 目录")
    parser.add_argument("--endpoint", required=True, help="已启动 Worker endpoint，例如 http://127.0.0.1:26200")
    parser.add_argument("--output", type=Path, required=True, help="验收 evidence 输出目录")
    parser.add_argument("--version", help="可选；默认读取 proxy @v/list 的最后一个版本")
    parser.add_argument("--shared-memory", action="store_true", help="连接 unix:// endpoint 时打开 SDK SHM 模式")
    args = parser.parse_args()

    proxy = args.proxy.resolve()
    version = args.version or latest_version(proxy)
    args.output.mkdir(parents=True, exist_ok=True)
    log = args.output / "go-sdk-artifact-consumer.log"
    log.write_text("", encoding="utf-8")

    with tempfile.TemporaryDirectory(prefix="dms-go-sdk-consumer-") as tmp:
        app = Path(tmp)
        write_consumer(app, version)
        assert_no_local_dependency(app)

        env = consumer_env(os.environ, proxy, args.endpoint, args.shared_memory, Path(tmp))

        run(["go", "mod", "download", "-json", MODULE], app, env, log)
        run(["go", "mod", "tidy"], app, env, log)
        assert_no_local_dependency(app)
        run(["go", "list", "-m", "-json", "all"], app, env, log)
        run(["go", "run", "."], app, env, log)

        module_json = subprocess.check_output(["go", "list", "-m", "-json", MODULE], cwd=app, env=env)
        (args.output / "go-sdk-module.json").write_bytes(module_json)
        module_info = json.loads(module_json)
        if "Replace" in module_info:
            raise RuntimeError("Go SDK dependency unexpectedly used replace")
        if not str(module_info.get("Dir", "")).startswith(str(Path(env["GOMODCACHE"]))):
            raise RuntimeError(f"Go SDK was not loaded from isolated module cache: {module_info.get('Dir')}")
        shutil.copy2(app / "go.mod", args.output / "consumer.go.mod")
        shutil.copy2(app / "go.sum", args.output / "consumer.go.sum")

    result = {
        "status": "passed",
        "module": MODULE,
        "version": version,
        "proxy": proxy.as_uri(),
        "endpoint": args.endpoint,
        "shared_memory": args.shared_memory,
        "evidence": {
            "log": str(log),
            "go_mod": str(args.output / "consumer.go.mod"),
            "module": str(args.output / "go-sdk-module.json"),
        },
    }
    (args.output / "go-sdk-artifact-consumer.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(result, ensure_ascii=False), flush=True)


CONSUMER_GO = r'''
package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	dms "github.com/lelezi257/dms/sdk/go"
)

func must(ok bool, format string, args ...any) {
	if !ok {
		panic(fmt.Sprintf(format, args...))
	}
}

func mustDmsKind(err error, kind dms.ErrorKind) {
	var dmsErr *dms.DmsError
	must(errors.As(err, &dmsErr), "expected DmsError, got %T %v", err, err)
	must(dmsErr.Kind == kind, "expected kind %s, got %s: %v", kind, dmsErr.Kind, dmsErr)
}

func main() {
	ctx := context.Background()
	shared := strings.EqualFold(os.Getenv("DMS_SHARED_MEMORY"), "true")
	client, err := dms.Connect(ctx, os.Getenv("DMS_ENDPOINT"), dms.ClientOptions{
		SharedMemory: &shared,
		Timeout:      5 * time.Second,
	})
	if err != nil {
		panic(err)
	}
	defer client.Close()

	prefix := fmt.Sprintf("artifact/%d/", time.Now().UnixNano())
	verifyKV(ctx, client, prefix)
	verifyReaderAndBuffer(ctx, client, prefix)
	verifyBatchAndScan(ctx, client, prefix)
	verifyHash(ctx, client, prefix)
	verifyStableErrors(ctx, client, prefix)
	fmt.Println("go sdk artifact consumer passed")
}

func verifyKV(ctx context.Context, client *dms.Client, prefix string) {
	result, err := client.Set(ctx, prefix+"kv", []byte("hello"))
	must(err == nil && result.Len == 5, "Set result=%+v err=%v", result, err)
	got, found, err := client.Get(ctx, prefix+"kv")
	must(err == nil && found && string(got) == "hello", "Get got=%q found=%v err=%v", got, found, err)

	rangeResult, err := client.SetRange(ctx, prefix+"kv", 1, []byte("ABC"))
	must(err == nil && rangeResult.Len == 5, "SetRange result=%+v err=%v", rangeResult, err)
	got, found, err = client.Get(ctx, prefix+"kv")
	must(err == nil && found && string(got) == "hABCo", "Get after SetRange got=%q found=%v err=%v", got, found, err)

	info, found, err := client.Stat(ctx, prefix+"kv")
	must(err == nil && found && info.Key == prefix+"kv" && info.Len == 5, "Stat info=%+v found=%v err=%v", info, found, err)

	deleted, err := client.Del(ctx, prefix+"kv")
	must(err == nil && deleted.Deleted, "Del result=%+v err=%v", deleted, err)
	_, found, err = client.Get(ctx, prefix+"kv")
	must(err == nil && !found, "Get deleted found=%v err=%v", found, err)
}

func verifyReaderAndBuffer(ctx context.Context, client *dms.Client, prefix string) {
	large := bytes.Repeat([]byte("x"), 128*1024)
	large[0], large[len(large)-1] = 'A', 'Z'
	set, err := client.SetFrom(ctx, prefix+"large", bytes.NewReader(large), uint64(len(large)), dms.SetOptions{})
	must(err == nil && set.Len == uint64(len(large)), "SetFrom result=%+v err=%v", set, err)

	buf := make([]byte, 16)
	into, found, err := client.GetInto(ctx, prefix+"large", buf, dms.GetOptions{Range: &dms.ByteRange{Offset: 0, Len: 16}})
	must(err == nil && found && into.Len == 16 && buf[0] == 'A', "GetInto result=%+v found=%v buf=%q err=%v", into, found, buf, err)

	reader, found, err := client.GetReader(ctx, prefix+"large", dms.GetOptions{Range: &dms.ByteRange{Offset: uint64(len(large) - 8), Len: 8}})
	must(err == nil && found && reader.Len == 8, "GetReader result=%+v found=%v err=%v", reader, found, err)
	tail, err := io.ReadAll(reader.Body)
	closeErr := reader.Body.Close()
	must(err == nil && closeErr == nil && len(tail) == 8 && tail[7] == 'Z', "reader tail=%q readErr=%v closeErr=%v", tail, err, closeErr)
}

func verifyBatchAndScan(ctx context.Context, client *dms.Client, prefix string) {
	mset, err := client.MSet(ctx, []dms.KVEntry{
		{Key: prefix + "scan/a", Value: []byte("a")},
		{Key: prefix + "scan/b", Value: []byte("b")},
		{Key: prefix + "scan/c", Value: []byte("c")},
	}, dms.MSetOptions{})
	must(err == nil && len(mset.Versions) == 3, "MSet result=%+v err=%v", mset, err)

	mget, err := client.MGet(ctx, []string{prefix + "scan/a", prefix + "scan/missing", prefix + "scan/c"})
	must(err == nil && len(mget) == 3 && mget[0] != nil && mget[1] == nil && mget[2] != nil, "MGet result=%+v err=%v", mget, err)

	first, err := client.Scan(ctx, prefix+"scan/", dms.ScanOptions{Limit: 2})
	must(err == nil && len(first.Items) == 2 && first.NextCursor != "", "first Scan=%+v err=%v", first, err)
	second, err := client.Scan(ctx, prefix+"scan/", dms.ScanOptions{Cursor: first.NextCursor})
	must(err == nil && len(second.Items) == 1 && second.NextCursor == "", "second Scan=%+v err=%v", second, err)
}

func verifyHash(ctx context.Context, client *dms.Client, prefix string) {
	set, err := client.HSet(ctx, prefix+"hash", []dms.HashEntry{
		{Field: "f1", Value: []byte("abc")},
		{Field: "f2", Value: []byte("def")},
		{Field: "f3", Value: []byte("ghi")},
	}, dms.HashWriteOptions{})
	must(err == nil && set.FieldCount == 3, "HSet result=%+v err=%v", set, err)

	one, found, err := client.HGet(ctx, prefix+"hash", "f1")
	must(err == nil && found && string(one.Bytes) == "abc", "HGet value=%+v found=%v err=%v", one, found, err)

	selected, err := client.HMGet(ctx, prefix+"hash", []string{"f1", "missing", "f3"}, dms.HashGetOptions{})
	must(err == nil && len(selected.Values) == 3 && selected.Values[0] != nil && selected.Values[1] == nil && selected.Values[2] != nil, "HMGet=%+v err=%v", selected, err)

	all, err := client.HGetAll(ctx, prefix+"hash", dms.HashGetOptions{})
	must(err == nil && len(all.Entries) == 3, "HGetAll=%+v err=%v", all, err)

	scan, err := client.HScan(ctx, prefix+"hash", 0, dms.HashScanOptions{Limit: 2})
	must(err == nil && len(scan.Entries) == 2 && scan.NextCursor != 0, "HScan=%+v err=%v", scan, err)

	write, err := client.HWriteAt(ctx, prefix+"hash", "f1", 1, []byte("ZZ"), dms.HashRangeWriteOptions{})
	must(err == nil && write.Len == 3, "HWriteAt=%+v err=%v", write, err)
	one, found, err = client.HGet(ctx, prefix+"hash", "f1")
	must(err == nil && found && string(one.Bytes) == "aZZ", "HGet after HWriteAt=%+v found=%v err=%v", one, found, err)

	del, err := client.HDel(ctx, prefix+"hash", []string{"f2"}, dms.HashDeleteOptions{})
	must(err == nil && del.FieldCount == 2, "HDel=%+v err=%v", del, err)
}

func verifyStableErrors(ctx context.Context, client *dms.Client, prefix string) {
	_, err := dms.ConnectWithOptions(ctx, dms.ClientOptions{Endpoint: "127.0.0.1:1", TLS: "required"})
	mustDmsKind(err, dms.ErrorKindUnimplemented)

	_, err = client.Set(ctx, strings.Repeat("k", dms.MaxKeyLen+1), []byte("x"))
	mustDmsKind(err, dms.ErrorKindInvalidArgument)

	tiny := make([]byte, 2)
	_, found, err := client.GetInto(ctx, "missing", tiny, dms.GetOptions{})
	must(err == nil && !found, "GetInto missing found=%v err=%v", found, err)

	short := make([]byte, 1)
	shortBufferKey := prefix + "short-buffer"
	_, found, err = client.GetInto(ctx, shortBufferKey, short, dms.GetOptions{})
	must(err == nil && !found, "precondition key should be absent before short-buffer check")
	_, err = client.Set(ctx, shortBufferKey, []byte("long"))
	must(err == nil, "Set short-buffer fixture: %v", err)
	_, found, err = client.GetInto(ctx, shortBufferKey, short, dms.GetOptions{})
	must(found, "short-buffer key must exist")
	mustDmsKind(err, dms.ErrorKindInvalidArgument)
}
'''


if __name__ == "__main__":
    main()
