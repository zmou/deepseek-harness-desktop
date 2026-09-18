#!/usr/bin/env bash
# ============================================================
# DeepSeek Harness Desktop - macOS 一键打包脚本
# 用法:  bash scripts/build-mac.sh
# 产物:  tauri-app/src-tauri/target/release/bundle/dmg/DeepSeek-Harness-Desktop_<version>_<arch>.dmg
#        （强压缩重压后的发布用 dmg，<arch> 为 aarch64 或 x64）
#         tauri-app/src-tauri/target/release/bundle/macos/*.app
# ============================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail() { echo "错误: $1"; exit 1; }

echo "=============================================="
echo " DeepSeek Harness Desktop - macOS 打包"
echo "=============================================="

# ---------- [1/5] 检查依赖 ----------
echo ""
echo "[1/5] 检查依赖..."
command -v node    >/dev/null 2>&1 || fail "未找到 node，请先安装: brew install node"
command -v npm     >/dev/null 2>&1 || fail "未找到 npm"
command -v cargo   >/dev/null 2>&1 || fail "未找到 cargo/rust，请先安装: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
command -v hdiutil >/dev/null 2>&1 || fail "未找到 hdiutil（应随 macOS 内置）"
xcode-select -p   >/dev/null 2>&1 || fail "未安装 Xcode Command Line Tools，请运行: xcode-select --install"

echo "  node : $(node -v)"
echo "  npm  : $(npm -v)"
echo "  cargo: $(cargo --version | head -1)"
echo "  Xcode CLT: OK"

# ---------- [2/5] 构建 runtime ----------
echo ""
echo "[2/5] 构建 runtime（下载 darwin node + 安装 dsh + 应用 glob 补丁）..."
echo "      首次运行需联网下载 node 与 npm 安装依赖，可能耗时数分钟..."
node scripts/build-runtime.mjs

# ---------- [3/5] 打包 ----------
echo ""
echo "[3/5] 打包 macOS 应用（app + dmg）..."
cd tauri-app/src-tauri
npx --yes @tauri-apps/cli build
cd "$ROOT"

# ---------- [4/5] 用 hdiutil 强压缩重建 dmg ----------
echo ""
echo "[4/5] 强压缩重建 dmg（LZMA/ULMO，约比 Tauri 默认 UDZO 再小 30~50%）..."

CONF_PATH="$ROOT/tauri-app/src-tauri/tauri.conf.json"
VERSION="$(node -p "require('$CONF_PATH').version")"
ARCH="$(uname -m)"
if [ "$ARCH" = "arm64" ]; then ARCH_LABEL="aarch64"; else ARCH_LABEL="x64"; fi

APP_DIR="$ROOT/tauri-app/src-tauri/target/release/bundle/macos"
DMG_DIR="$ROOT/tauri-app/src-tauri/target/release/bundle/dmg"
APP="$(ls -dt "$APP_DIR"/*.app 2>/dev/null | head -1 || true)"
[ -n "$APP" ] || fail "未找到 .app 产物: $APP_DIR"

# 命名与 README 下载表一致：DeepSeek-Harness-Desktop_<version>_<arch>.dmg
OUT_DMG="$DMG_DIR/DeepSeek-Harness-Desktop_${VERSION}_${ARCH_LABEL}.dmg"
# 临时工作区：staging 放镜像内容，rw 镜像必须放在 staging 目录**外面**。
# 踩坑记录：若把输出镜像写进 `-srcfolder` 内部（`$STAGE_DIR/rw.dmg`），hdiutil 会挂载新
# 镜像后把 `$STAGE_DIR` 的内容整个拷进去，而 rw 镜像自己就在这个目录里 —— 边拷边长大，
# 变成自我引用式膨胀，最终报 `hdiutil: create failed - 结果太大`（实测另见
# `设备上无剩余空间`），脚本在 `set -e` 下直接中断，永远拿不到最终 dmg。
WORK_DIR="$(mktemp -d)"
STAGE_DIR="$WORK_DIR/stage"
RW_DMG="$WORK_DIR/rw.dmg"
# `|| true`：某些 IDE/宿主会注入 rm 包装脚本拦截批量删除，清理失败不应覆盖构建结论
trap 'rm -rf "$WORK_DIR" 2>/dev/null || true' EXIT
mkdir -p "$STAGE_DIR"

rm -f "$OUT_DMG"

# 组装镜像内容：.app + Applications 快捷方式（保留 Tauri 默认 dmg 的拖拽安装体验）
cp -R "$APP" "$STAGE_DIR/"
ln -s /Applications "$STAGE_DIR/Applications"

# 分两步：
# 1) 不压缩的读写镜像（只做“拷入 .app + 布局”，快，秒级）
# 2) convert 阶段再做 LZMA 强压缩（ULMO）——整个镜像统一压缩，效果远好于 UDZO。
#    ULMO 与 Tauri 默认的 UDZO 同为合法的 hdiutil 格式，任何 macOS 10.13+ 都支持挂载。
hdiutil create -volname "DeepSeek Harness Desktop" -srcfolder "$STAGE_DIR" -ov -format UDRW "$RW_DMG" >/dev/null
if ! hdiutil convert "$RW_DMG" -format ULMO -o "$OUT_DMG" >/dev/null 2>&1; then
  # 极旧系统兜底：ULMO 不可用时退回 UDBZ(bzip2)
  rm -f "$OUT_DMG"   # 清掉失败可能留下的半成品，否则 convert 会因目标已存在而报错
  hdiutil convert "$RW_DMG" -format UDBZ -o "$OUT_DMG" >/dev/null
fi

# 删除 Tauri 默认命名的 dmg，避免同名混淆。
# 用 if 而不是 `[ ... ] && rm`：后者在“最后一个元素恰好不需要删”时会以非 0 状态结束
# 整个 while，配合 `set -o pipefail` 会直接中断脚本（在阶段 5 汇总输出之前，实测踩过）。
for f in "$DMG_DIR"/*.dmg; do
  if [ -e "$f" ] && [ "$(basename "$f")" != "$(basename "$OUT_DMG")" ]; then
    rm -f "$f"
  fi
done

echo "  强压缩 dmg: $OUT_DMG"
ls -lh "$OUT_DMG" | awk '{print "  大小: "$5}'

# ---------- [5/5] 输出结果 ----------
echo ""
echo "[5/5] 完成！"
ls -lh "$OUT_DMG" >/dev/null 2>&1 && echo "  DMG: $OUT_DMG"
echo "  APP: $APP"
echo ""
echo "提示: 未配置 Apple 开发者签名证书，产物为 ad-hoc 签名。"
echo "      首次打开若被 Gatekeeper 拦截，请右键应用 -> 打开。"
echo "      如需正式分发，请配置签名证书后重新打包。"
