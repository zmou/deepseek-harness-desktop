#!/usr/bin/env bash
# ============================================================
# DeepSeek Harness Desktop — Electron 兼容版（macOS 12 及以下）一键打包
#
# 用法:  bash scripts/build-mac-electron.sh
#         DSH_VERSION=0.1.6 bash scripts/build-mac-electron.sh   # 覆盖 dsh 版本
#
# 产物:  electron-app/release/DeepSeek-Harness-Desktop_v<version>_<arch>-electron.dmg
#        （<arch> 为 aarch64 或 x64；与 Tauri 默认版 dmg 靠 -electron 后缀区分）
#
# 与 Tauri 版链路完全独立：仅复用 scripts/build-runtime.mjs 的运行时产物。
# 规格依据：docs/electron-macos-compat-spec.md §9 / §11.3
# ============================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

APP_DIR="$ROOT/electron-app"
RELEASE_DIR="$APP_DIR/release"
# 门槛守护目标：Electron 43 自身的 LSMinimumSystemVersion 必须仍是 macOS 12
MAX_MIN_SYSTEM_VERSION="12.0"

fail() { echo "错误: $1"; exit 1; }

echo "=============================================="
echo " DeepSeek Harness Desktop - Electron 兼容版打包"
echo "=============================================="

# ---------- [1/7] 检查依赖 ----------
echo ""
echo "[1/7] 检查依赖..."
command -v node    >/dev/null 2>&1 || fail "未找到 node，请先安装: brew install node"
command -v npm     >/dev/null 2>&1 || fail "未找到 npm"
command -v hdiutil >/dev/null 2>&1 || fail "未找到 hdiutil（应随 macOS 内置）"
echo "  node : $(node -v)"
echo "  npm  : $(npm -v)"

# ---------- [2/7] 构建 runtime（与 Tauri 版共享产物） ----------
echo ""
echo "[2/7] 构建 runtime（下载 darwin node + 安装 dsh + 应用 glob 补丁）..."
echo "      首次运行需联网下载 node 与 npm 依赖，可能耗时数分钟；已存在则增量跳过..."
node scripts/build-runtime.mjs

# ---------- [3/7] 安装 Electron 依赖 ----------
echo ""
echo "[3/7] 安装 electron-app 依赖（electron 精确锁 43.x，防止门槛被抬高）..."
cd "$APP_DIR"
npm install --no-audit --no-fund

# ---------- [4/7] 同步版本号（dsh 版本 + -electron 后缀） ----------
echo ""
echo "[4/7] 同步版本号..."
# 版本源与 build-runtime.mjs 保持一致；DSH_VERSION 环境变量可覆盖（升级 dsh 时联动）
# 注意：不要接 `| head -1`——head 提前退出会让 sed 收到 SIGPIPE，
# 在 `set -o pipefail` 下会被当成失败（该行在文件里唯一，无需截断）。
BUILD_RUNTIME_VERSION="$(sed -n "s/^const DSH_VERSION = process\.env\.DSH_VERSION || '\([^']*\)'$/\1/p" "$ROOT/scripts/build-runtime.mjs" || true)"
[ -n "$BUILD_RUNTIME_VERSION" ] || fail "无法从 scripts/build-runtime.mjs 解析 DSH_VERSION"
DSH_VERSION="${DSH_VERSION:-$BUILD_RUNTIME_VERSION}"
PACKAGE_VERSION="${DSH_VERSION}-electron"

node -e '
  const fs = require("node:fs")
  const path = "package.json"
  const pkg = JSON.parse(fs.readFileSync(path, "utf8"))
  pkg.version = process.argv[1]
  fs.writeFileSync(path, JSON.stringify(pkg, null, 2) + "\n")
' "$PACKAGE_VERSION"
echo "  dsh 版本   : $DSH_VERSION"
echo "  app 版本号 : $PACKAGE_VERSION"

# ---------- [5/7] 打包 ----------
echo ""
echo "[5/7] 打包 Electron 应用（app + dmg）..."
# 只清掉上次的 dmg 等少量产物；.app 目录交给 electron-builder 自己重建。
# 注意：electron-builder 会删除并重建 `release/mac*/`，这一步依赖文件系统的批量删除能力。
# 若宿主（IDE/编辑器）通过 NODE_OPTIONS 向子进程注入了删除安全钩子（--require shim），
# 该步可能报 SAFE_DELETE_BULK_CONFIRM_REQUIRED；此时手动删除 electron-app/release/ 后重跑即可。
if [[ "${NODE_OPTIONS:-}" == *"--require"* ]]; then
  echo "  提示: 检测到宿主注入的 NODE_OPTIONS shim，若打包报批量删除被拦截，请先手动删除 electron-app/release/"
fi
rm -f "$RELEASE_DIR"/*.dmg "$RELEASE_DIR"/*.dmg.blockmap 2>/dev/null || true
npx electron-builder --mac dmg --publish never

# ---------- [6/7] 门槛守护 ----------
echo ""
echo "[6/7] 校验产物最低系统版本（必须 <= ${MAX_MIN_SYSTEM_VERSION}）..."
APP="$(ls -dt "$RELEASE_DIR"/mac*/*.app 2>/dev/null | head -1 || true)"
[ -n "$APP" ] || fail "未找到 .app 产物: $RELEASE_DIR/mac*/*.app"

# 逐个检查 bundle 里出现的 LSMinimumSystemVersion，取最严格的一个。
# 实测 Electron 43 的该键在 `Contents/Info.plist`（Electron Framework 内不再声明）。
MIN_VER="$(node -e '
  const { execFileSync } = require("node:child_process")
  const fs = require("node:fs")
  const path = require("node:path")
  const app = process.argv[1]
  const candidates = [
    path.join(app, "Contents", "Info.plist"),
    path.join(app, "Contents", "Frameworks", "Electron Framework.framework", "Resources", "Info.plist"),
  ].filter((p) => fs.existsSync(p))
  const parse = (v) => v.split(".").map((n) => Number.parseInt(n, 10) || 0)
  const cmp = (a, b) => (a[0] - b[0]) || ((a[1] || 0) - (b[1] || 0))
  let max = null
  for (const plist of candidates) {
    let out = ""
    try {
      out = execFileSync("plutil", ["-p", plist], { encoding: "utf8" })
    } catch { continue }
    const m = out.match(/"LSMinimumSystemVersion"\s*=>\s*"([^"]+)"/)
    if (!m) continue
    if (max === null || cmp(parse(m[1]), parse(max)) > 0) max = m[1]
  }
  if (max === null) { console.error("LSMinimumSystemVersion not found in " + app); process.exit(2) }
  process.stdout.write(max)
' "$APP")" || fail "无法读取产物的 LSMinimumSystemVersion（门槛断言失败）"

node -e '
  const got = process.argv[1]
  const max = process.argv[2]
  const parse = (v) => v.split(".").map((n) => Number.parseInt(n, 10) || 0)
  const cmp = (a, b) => (a[0] - b[0]) || ((a[1] || 0) - (b[1] || 0))
  if (cmp(parse(got), parse(max)) > 0) {
    console.error(`\n门槛断言失败: LSMinimumSystemVersion=${got} > ${max}`)
    console.error("说明依赖锁解析到了更高的 Electron 大版本，macOS 12 用户将无法安装。")
    console.error("请检查 package.json 里 electron 是否仍是精确锁定的 43.x。")
    process.exit(1)
  }
  console.log(`  LSMinimumSystemVersion = ${got}（<= ${max}，通过）`)
' "$MIN_VER" "$MAX_MIN_SYSTEM_VERSION"

# ---------- [7/7] 重命名 dmg ----------
echo ""
echo "[7/7] 规范化 dmg 文件名..."
ARCH="$(uname -m)"
if [ "$ARCH" = "arm64" ]; then ARCH_LABEL="aarch64"; else ARCH_LABEL="x64"; fi

SRC_DMG="$(ls -t "$RELEASE_DIR"/*.dmg 2>/dev/null | head -1 || true)"
[ -n "$SRC_DMG" ] || fail "未找到 dmg 产物: $RELEASE_DIR/*.dmg"

OUT_DMG="$RELEASE_DIR/DeepSeek-Harness-Desktop_v${DSH_VERSION}_${ARCH_LABEL}-electron.dmg"
if [ "$SRC_DMG" != "$OUT_DMG" ]; then
  mv -f "$SRC_DMG" "$OUT_DMG"
fi
# 删除 electron-builder 的默认命名（同名冲突或上次残留），避免混淆
for f in "$RELEASE_DIR"/*.dmg; do
  if [ -e "$f" ] && [ "$(basename "$f")" != "$(basename "$OUT_DMG")" ]; then
    rm -f "$f"
  fi
done

echo ""
echo "=============================================="
echo " 完成！"
echo "  DMG: $OUT_DMG"
ls -lh "$OUT_DMG" | awk '{print "  大小: "$5}'
echo "  APP: $APP"
echo ""
echo "提示: 默认 ad-hoc 签名。首次打开若被 Gatekeeper 拦截，请右键应用 -> 打开。"
echo "      Electron 43 是官方支持 macOS 12 的最后一档，安全更新会逐步停止（规格 §13 R4）。"
echo "=============================================="
