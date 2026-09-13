#!/usr/bin/env bash
# ============================================================
# DeepSeek Harness Desktop - macOS 一键打包脚本
# 用法:  bash scripts/build-mac.sh
# 产物:  tauri-app/src-tauri/target/release/bundle/dmg/*.dmg
#         tauri-app/src-tauri/target/release/bundle/macos/*.app
# ============================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "=============================================="
echo " DeepSeek Harness Desktop - macOS 打包"
echo "=============================================="

# ---------- [1/4] 检查依赖 ----------
echo ""
echo "[1/4] 检查依赖..."
fail() { echo "错误: $1"; exit 1; }

command -v node  >/dev/null 2>&1 || fail "未找到 node，请先安装: brew install node"
command -v npm   >/dev/null 2>&1 || fail "未找到 npm"
command -v cargo >/dev/null 2>&1 || fail "未找到 cargo/rust，请先安装: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
xcode-select -p >/dev/null 2>&1 || fail "未安装 Xcode Command Line Tools，请运行: xcode-select --install"

echo "  node : $(node -v)"
echo "  npm  : $(npm -v)"
echo "  cargo: $(cargo --version | head -1)"
echo "  Xcode CLT: OK"

# ---------- [2/4] 构建 runtime ----------
echo ""
echo "[2/4] 构建 runtime（下载 darwin node + 安装 dsh + 应用 glob 补丁）..."
echo "      首次运行需联网下载 node 与 npm 安装依赖，可能耗时数分钟..."
node scripts/build-runtime.mjs

# ---------- [3/4] 打包 ----------
echo ""
echo "[3/4] 打包 macOS 应用（app + dmg）..."
cd tauri-app/src-tauri
npx --yes @tauri-apps/cli build

# ---------- [4/4] 输出结果 ----------
echo ""
echo "[4/4] 完成！"
DMG="$(ls -t target/release/bundle/dmg/*.dmg 2>/dev/null | head -1 || true)"
APP="$(ls -dt target/release/bundle/macos/*.app 2>/dev/null | head -1 || true)"
if [ -n "$DMG" ]; then echo "  DMG: $ROOT/tauri-app/src-tauri/$DMG"; fi
if [ -n "$APP" ]; then echo "  APP: $ROOT/tauri-app/src-tauri/$APP"; fi
echo ""
echo "提示: 未配置 Apple 开发者签名证书，产物为 ad-hoc 签名。"
echo "      首次打开若被 Gatekeeper 拦截，请右键应用 -> 打开。"
echo "      如需正式分发，请配置签名证书后重新打包。"
