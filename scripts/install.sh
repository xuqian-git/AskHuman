#!/usr/bin/env bash
# 构建并安装 AskHuman 到 ~/.local/bin（macOS / Linux）。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
cd "$REPO_ROOT"

if ! command -v pnpm >/dev/null 2>&1; then
  echo "错误: 需要 pnpm（npm i -g pnpm）" >&2
  exit 1
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "错误: 需要 Rust 工具链（https://rustup.rs）" >&2
  exit 1
fi

# 在途请求提示：daemon 正服务中的提问不会被安装打断——换新会在它们完结后自动发生（graceful drain），
# 期间新提问会等待。此处只提示，不强杀。
if command -v AskHuman >/dev/null 2>&1; then
  ACTIVE="$(AskHuman daemon status 2>/dev/null | sed -n 's/.*requests[[:space:]]*\([0-9][0-9]*\) active.*/\1/p' | head -n1 || true)"
  if [ -n "${ACTIVE:-}" ] && [ "$ACTIVE" -gt 0 ] 2>/dev/null; then
    echo "提示: daemon 当前有 $ACTIVE 个在途请求；安装后将在它们完结后自动换新（期间新提问会等待）。"
    echo "      立即换新: AskHuman daemon restart --force（会打断在途请求）"
  fi
fi

echo "==> 安装前端依赖"
pnpm install

echo "==> 构建前端 (dist/)"
pnpm build

echo "==> 编译 release 二进制（前端资源在此步骤被嵌入）"
# --features custom-protocol：生产构建必须启用，否则二进制以 dev 模式连 devUrl 导致白屏。
cargo build --release --manifest-path src-tauri/Cargo.toml --features custom-protocol

BIN_PATH="src-tauri/target/release/AskHuman"
if [ ! -f "$BIN_PATH" ]; then
  echo "错误: 未找到编译产物 $BIN_PATH" >&2
  exit 1
fi

echo "==> 安装到 $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
cp "$BIN_PATH" "$INSTALL_DIR/AskHuman"
chmod 0755 "$INSTALL_DIR/AskHuman"

if [ "$(uname)" = "Darwin" ]; then
  # 清除 quarantine，降低拷贝后被 Gatekeeper 拦截的概率
  xattr -d com.apple.quarantine "$INSTALL_DIR/AskHuman" 2>/dev/null || true
  # Sign with a stable identity + fixed identifier so the OS keychain trusts the binary across
  # rebuilds (its designated requirement is cdhash-independent) → secret reads stay prompt-free.
  # Identity: $CODESIGN_IDENTITY if set, else auto-detect the first local codesigning cert,
  # else ad-hoc (which falls back to per-build keychain prompts).
  IDENTITY="${CODESIGN_IDENTITY:-}"
  if [ -z "$IDENTITY" ]; then
    IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null | awk '/^[[:space:]]*[0-9]+\)/{print $2; exit}')"
  fi
  [ -z "$IDENTITY" ] && IDENTITY="-"
  if [ "$IDENTITY" = "-" ]; then
    echo "==> 签名 (ad-hoc; 设置 CODESIGN_IDENTITY 可避免每次重装的钥匙串弹框)"
  else
    echo "==> 签名 (identity: $IDENTITY, identifier: com.naituw.humaninloop)"
  fi
  codesign -i com.naituw.humaninloop --force --sign "$IDENTITY" "$INSTALL_DIR/AskHuman" || \
    echo "警告: 签名失败，已跳过" >&2
fi

echo "==> 完成：$INSTALL_DIR/AskHuman"
if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
  echo "提示: $INSTALL_DIR 不在 PATH 中，请将其加入 PATH。"
fi
