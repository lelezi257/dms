#!/usr/bin/env bash
# 平台无关的 CI 前置门禁；不启动/停止用户服务，不执行远端发布。
set -euo pipefail
SOURCE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
if [[ "$(uname -s)" != Linux ]]; then
  echo "请先进入 Linux 构建环境再运行此门禁" >&2
  exit 2
fi
cd "${SOURCE_DIR}"
export PYTHONDONTWRITEBYTECODE=1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 -m unittest discover -s scripts/release -p 'test_*.py'
python3 -m unittest discover -s scripts -p 'test_package_sdk.py'
python3 -m unittest discover -s scripts/docs -p 'test_*.py'
echo "源码门禁通过；还需候选包与隔离安装验收，不能将此结果当成已发布。"
