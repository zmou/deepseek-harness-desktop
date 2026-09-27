<#
.SYNOPSIS
    DeepSeek Harness Desktop - Windows 一键打包脚本（只打包，不安装）

.DESCRIPTION
    一键完成：依赖检查 -> 版本同步 -> 构建 runtime -> 打包 NSIS 安装包。

    用法（在项目根目录执行）：
      powershell -ExecutionPolicy Bypass -File scripts/build-win.ps1
      powershell -ExecutionPolicy Bypass -File scripts/build-win.ps1 -DshVersion 0.1.2-rc.1

.PARAMETER DshVersion
    可选。要打包的 dsh 版本（如 0.1.2-rc.1）。
    指定后会同步写入 scripts/build-runtime.mjs、tauri.conf.json、Cargo.toml。
    留空则沿用 build-runtime.mjs 中现有的 DSH_VERSION（只重新构建 + 打包）。

.NOTES
    本脚本刻意不使用正则（避免 PowerShell 解析歧义），全部用 IndexOf/Substring 字符串操作。
#>
param(
    [string]$DshVersion = ''
)

$ErrorActionPreference = 'Stop'
$ROOT = Split-Path -Parent $PSScriptRoot
Set-Location $ROOT

function Fail {
    param([string]$msg)
    Write-Host ('错误: ' + $msg) -ForegroundColor Red
    exit 1
}

# 无 BOM UTF-8 写入：PowerShell 5.1 的 Set-Content -Encoding UTF8 会写入 BOM（会破坏
# JSON/JS 解析），默认 ANSI(GBK) 又会把 UTF-8 中文注释不可逆地变成乱码
# （build-runtime.mjs 曾因此出现乱码）。因此读写一律走 .NET API，显式无 BOM UTF-8。
$Utf8NoBom = [System.Text.UTF8Encoding]::new($false)

Write-Host '==============================================' -ForegroundColor Cyan
Write-Host ' DeepSeek Harness Desktop - Windows 打包' -ForegroundColor Cyan
Write-Host '==============================================' -ForegroundColor Cyan

# ---------- [1/5] 检查依赖 ----------
Write-Host ''
Write-Host '[1/5] 检查依赖...' -ForegroundColor Yellow
foreach ($cmd in @('node', 'npm', 'cargo')) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
        Fail "未找到 $cmd ，请先安装并加入 PATH"
    }
}
$tauriCli = (& npx --no-install @tauri-apps/cli --version 2>$null | Select-Object -First 1)
if (-not $tauriCli) {
    Fail '未找到 @tauri-apps/cli，请先安装: npm install -g @tauri-apps/cli'
}
$nodeVer = (node -v)
$cargoVer = (cargo --version)
Write-Host ('  node      : ' + $nodeVer)
Write-Host ('  cargo     : ' + $cargoVer)
Write-Host ('  tauri-cli : ' + $tauriCli)

# ---------- [2/5] 版本同步 ----------
$brPath    = Join-Path $ROOT 'scripts/build-runtime.mjs'
$confPath  = Join-Path $ROOT 'tauri-app/src-tauri/tauri.conf.json'
$cargoPath = Join-Path $ROOT 'tauri-app/src-tauri/Cargo.toml'

Write-Host ''
if ($DshVersion -ne '') {
    Write-Host "[2/5] 同步版本号 -> $DshVersion ..." -ForegroundColor Yellow
    foreach ($p in @($brPath, $confPath, $cargoPath)) {
        if (-not (Test-Path $p)) { Fail "文件不存在: $p" }
    }

    # --- build-runtime.mjs: 定位 DSH_VERSION 默认值并替换 ---
    $br = [System.IO.File]::ReadAllText($brPath)
    $keyBr = 'process.env.DSH_VERSION || ' + "'"
    $i = $br.IndexOf($keyBr)
    if ($i -lt 0) { Fail 'build-runtime.mjs 中未找到 DSH_VERSION 默认值' }
    $s = $i + $keyBr.Length
    $e = $br.IndexOf("'", $s)
    if ($e -lt 0) { Fail 'build-runtime.mjs 中 DSH_VERSION 引号不完整' }
    $brNew = $br.Substring(0, $s) + $DshVersion + $br.Substring($e)
    [System.IO.File]::WriteAllText($brPath, $brNew, $Utf8NoBom)

    # --- tauri.conf.json: 定位 version 字段并替换 ---
    $tc = [System.IO.File]::ReadAllText($confPath)
    $keyTc = '"version": "'
    $i = $tc.IndexOf($keyTc)
    if ($i -lt 0) { Fail 'tauri.conf.json 中未找到 version 字段' }
    $s = $i + $keyTc.Length
    $e = $tc.IndexOf('"', $s)
    if ($e -lt 0) { Fail 'tauri.conf.json 中 version 引号不完整' }
    $tcNew = $tc.Substring(0, $s) + $DshVersion + $tc.Substring($e)
    [System.IO.File]::WriteAllText($confPath, $tcNew, $Utf8NoBom)

    # --- Cargo.toml: 逐行改 [package] 下第一个 version（不影响依赖行）---
    $lines = [System.IO.File]::ReadAllLines($cargoPath)
    $done = $false
    for ($k = 0; $k -lt $lines.Count; $k++) {
        if ($lines[$k].TrimStart().StartsWith('version')) {
            $lines[$k] = 'version = "' + $DshVersion + '"'
            $done = $true
            break
        }
    }
    if (-not $done) { Fail 'Cargo.toml 中未找到 version 行' }
    [System.IO.File]::WriteAllLines($cargoPath, $lines, $Utf8NoBom)

    Write-Host '  已同步 build-runtime.mjs / tauri.conf.json / Cargo.toml' -ForegroundColor Gray
} else {
    Write-Host '[2/5] 未指定 -DshVersion，沿用现有版本' -ForegroundColor Yellow
    $br = [System.IO.File]::ReadAllText($brPath)
    $keyBr = 'process.env.DSH_VERSION || ' + "'"
    $i = $br.IndexOf($keyBr)
    if ($i -ge 0) {
        $s = $i + $keyBr.Length
        $e = $br.IndexOf("'", $s)
        if ($e -gt $s) {
            $curVer = $br.Substring($s, $e - $s)
            Write-Host "  当前版本: $curVer" -ForegroundColor Gray
        }
    }
}

# ---------- [3/5a] 构建 runtime 前：清理 runtime 工作区 ----------
# 原则：每次打包都从干净的 runtime 工作区开始，避免历史残留混入安装包。
# 只清 runtime 相关残留，不做 cargo clean（增量编译有指纹校验，不会污染产物，
# 但全量重编会把 4 分钟拉到 15-25 分钟）。
Write-Host ''
Write-Host '[3/5a] 清理 runtime 工作区...' -ForegroundColor Yellow

$resourcesDir = Join-Path $ROOT 'tauri-app/resources'
$runtimeDir   = Join-Path $resourcesDir 'runtime'
$backupDir    = Join-Path $resourcesDir '.dsh-runtime-backup'

# (a) runtime/ 内部的失败残留 dsh-runtime.failed-*：位于资源根内，
#     会被 Tauri 原样打进安装包（等于多打一份 runtime，体积翻倍）。
#     超长路径（实测最长 273 字符）需用 robocopy 空目录镜像绕过 MAX_PATH。
$failed = @(Get-ChildItem -LiteralPath $runtimeDir -Directory -Filter 'dsh-runtime.failed-*' -ErrorAction SilentlyContinue)
if ($failed.Count -gt 0) {
    $stub = Join-Path $env:TEMP ('dsh-empty-stub-' + $PID)
    New-Item -ItemType Directory -Force -Path $stub | Out-Null
    foreach ($f in $failed) {
        Write-Host ('  清除 runtime 内失败残留: ' + $f.Name) -ForegroundColor Gray
        & robocopy $stub $f.FullName /MIR /NFL /NDL /NJH /NJS /NC /NS /NP /MT:16 | Out-Null
        if ($LASTEXITCODE -gt 7) { Fail ('robocopy 清理失败: ' + $f.FullName) }
        Remove-Item -LiteralPath $f.FullName -Recurse -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $f.FullName) { Fail ('无法清除会污染安装包的残留: ' + $f.FullName) }
    }
    Remove-Item -LiteralPath $stub -Recurse -Force -ErrorAction SilentlyContinue
} else {
    Write-Host '  runtime/ 内无失败残留' -ForegroundColor Gray
}

# (b) 备份目录：位于 runtime/ 之外（不打包），只保留最新 1 份作回滚点。
if (Test-Path -LiteralPath $backupDir) {
    $backups = @(Get-ChildItem -LiteralPath $backupDir -Directory -Filter 'dsh-runtime.*' -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending)
    if ($backups.Count -gt 1) {
        $stub = Join-Path $env:TEMP ('dsh-empty-stub-' + $PID)
        New-Item -ItemType Directory -Force -Path $stub | Out-Null
        foreach ($b in ($backups | Select-Object -Skip 1)) {
            $sizeMB = [math]::Round((Get-ChildItem -LiteralPath $b.FullName -Recurse -File -ErrorAction SilentlyContinue |
                Measure-Object Length -Sum).Sum / 1MB, 1)
            Write-Host ('  清除旧备份: ' + $b.Name + ' (' + $sizeMB + ' MB)') -ForegroundColor Gray
            & robocopy $stub $b.FullName /MIR /NFL /NDL /NJH /NJS /NC /NS /NP /MT:16 | Out-Null
            if ($LASTEXITCODE -gt 7) { Fail ('robocopy 清理备份失败: ' + $b.FullName) }
            Remove-Item -LiteralPath $b.FullName -Recurse -Force -ErrorAction SilentlyContinue
        }
        Remove-Item -LiteralPath $stub -Recurse -Force -ErrorAction SilentlyContinue
        Write-Host ('  已保留最新备份: ' + $backups[0].Name) -ForegroundColor Gray
    } else {
        Write-Host '  备份目录无需清理' -ForegroundColor Gray
    }
}
# ---------- [3/5b] 构建 runtime ----------
Write-Host ''
Write-Host '[3/5b] 构建 runtime（node + npm install dsh + 应用 glob 补丁）...' -ForegroundColor Yellow
Write-Host '      首次或换版本时需联网下载依赖，可能耗时数分钟到十几分钟' -ForegroundColor Gray
& node $brPath
if ($LASTEXITCODE -ne 0) { Fail 'runtime 构建失败' }

# ---------- [4/5] 打包 ----------
Write-Host ''
Write-Host '[4/5] 打包 NSIS 安装包（编译 + 压缩 runtime，通常 20~45 分钟）...' -ForegroundColor Yellow
Push-Location (Join-Path $ROOT 'tauri-app/src-tauri')
try {
    & npx --no-install @tauri-apps/cli build
    $code = $LASTEXITCODE
} finally {
    Pop-Location
}
if ($code -ne 0) { Fail '打包失败，请查看构建输出' }

# ---------- [5/5] 输出结果 ----------
Write-Host ''
Write-Host '[5/5] 完成！' -ForegroundColor Green
$nsisDir = Join-Path $ROOT 'tauri-app/src-tauri/target/release/bundle/nsis'
$installer = Get-ChildItem (Join-Path $nsisDir '*.exe') -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($installer) {
    $sizeMB = [math]::Round($installer.Length / 1MB, 1)
    Write-Host ('  安装包  : ' + $installer.FullName)
    Write-Host ('  大小    : ' + $sizeMB + ' MB')
    Write-Host ('  修改时间: ' + $installer.LastWriteTime)

    # 规范化文件名：DeepSeek-Harness-Desktop-Setup_v<version>_<arch>.exe
    # （Tauri 默认生成 `DeepSeek Harness Desktop_<version>_<arch>-setup.exe`，
    #   含空格且分隔符不一致；这里按发布规范统一重命名）
    $ver = [System.IO.File]::ReadAllText((Join-Path $ROOT 'scripts/build-runtime.mjs'))
    $keyVer = 'process.env.DSH_VERSION || ' + "'"
    $vi = $ver.IndexOf($keyVer)
    $vs = $vi + $keyVer.Length
    $ve = $ver.IndexOf("'", $vs)
    $curVersion = $ver.Substring($vs, $ve - $vs)
    # arch：默认 x64；arm64 由构建机决定
    $arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }
    $newName = "DeepSeek-Harness-Desktop-Setup_v${curVersion}_${arch}.exe"
    if ($installer.Name -ne $newName) {
        $newPath = Join-Path $nsisDir $newName
        if (Test-Path -LiteralPath $newPath) { Remove-Item -LiteralPath $newPath -Force }
        Move-Item -LiteralPath $installer.FullName -Destination $newPath -Force
        Write-Host ('  规范化  : ' + $newName) -ForegroundColor Cyan
    }
} else {
    Write-Host "  未找到安装包产物，请检查 $nsisDir" -ForegroundColor Yellow
}
Write-Host ''
Write-Host '提示: 本脚本只打包，不安装。请手动运行上面的安装包。' -ForegroundColor Gray
