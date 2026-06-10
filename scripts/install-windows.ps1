# 构建并安装 AskHuman 到用户目录（Windows）。
$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptDir
$InstallDir = if ($env:INSTALL_DIR) { $env:INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\AskHuman" }
Set-Location $RepoRoot

if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
  Write-Error "需要 pnpm（npm i -g pnpm）"; exit 1
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  Write-Error "需要 Rust 工具链（https://rustup.rs）"; exit 1
}

# 在途请求提示（与 install.sh 对应；daemon 暂不支持 Windows 时 status 不可用，自然跳过）。
if (Get-Command AskHuman -ErrorAction SilentlyContinue) {
  $StatusOut = & AskHuman daemon status 2>$null
  if ($LASTEXITCODE -eq 0 -and $StatusOut -match 'requests\s+(\d+) active' -and [int]$Matches[1] -gt 0) {
    Write-Host "提示: daemon 当前有 $($Matches[1]) 个在途请求；安装后将在它们完结后自动换新（期间新提问会等待）。"
    Write-Host "      立即换新: AskHuman daemon restart --force（会打断在途请求）"
  }
}

Write-Host "==> 安装前端依赖"
pnpm install

Write-Host "==> 构建前端 (dist/)"
pnpm build

Write-Host "==> 编译 release（前端资源在此步骤被嵌入）"
# --features custom-protocol：生产构建必须启用，否则二进制以 dev 模式连 devUrl 导致白屏。
cargo build --release --manifest-path src-tauri/Cargo.toml --features custom-protocol

$Bin = "src-tauri\target\release\AskHuman.exe"
if (-not (Test-Path $Bin)) { Write-Error "未找到编译产物 $Bin"; exit 1 }

Write-Host "==> 安装到 $InstallDir"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item $Bin (Join-Path $InstallDir "AskHuman.exe") -Force

Write-Host "==> 完成：$InstallDir\AskHuman.exe"
Write-Host "提示: 请将 $InstallDir 加入 PATH 后即可在终端使用 AskHuman。"
