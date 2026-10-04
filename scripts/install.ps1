# Cage 一键安装（Windows）：从滚动 nightly Release 匿名直链下载二进制
# （不走 Actions artifacts、不带任何 token），校验 SHA256 后装入
# %LOCALAPPDATA%\cage，并把该目录追加进用户级 PATH。Linux / macOS 用
# install.sh。
#
# 用法（PowerShell）：
#   irm https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.ps1 | iex
#   .\install.ps1                 # 装最新 nightly
#   .\install.ps1 -Version v0.1.0 # 装指定 Release tag（需该 tag 的 Release 带对应平台资产）
#   .\install.ps1 -Uninstall      # 卸载
#
# 依赖：PowerShell 5.1+（Windows 自带），无需管理员权限。

param(
  [string]$Version = "nightly",
  [switch]$Uninstall
)

$ErrorActionPreference = "Stop"
# 旧版 PowerShell（5.1）默认可能未启用 TLS 1.2
try { [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12 } catch {}

$Repo = if ($env:CAGE_REPO) { $env:CAGE_REPO } else { "cuihairu/cage" }
$ExeName = "cage.exe"
$InstallDir = Join-Path $env:LOCALAPPDATA "cage"
$ExePath = Join-Path $InstallDir $ExeName

if ($Uninstall) {
  if (Test-Path $ExePath) {
    Remove-Item $ExePath -Force
    Write-Host "removed $ExePath"
  } else {
    Write-Error "install.ps1: $ExePath not found (nothing to uninstall)"
    exit 1
  }
  Write-Host "若安装目录已不在使用，可从用户级 PATH 移除 $InstallDir。"
  exit 0
}

# 平台与资产：仅 x86_64 试运行腿（ARM64 暂无资产）
if ($env:PROCESSOR_ARCHITECTURE -notmatch "AMD64") {
  Write-Error "install.ps1: 仅提供 Windows x86_64 资产（当前架构：$env:PROCESSOR_ARCHITECTURE）"
  exit 1
}
$Asset = "cage-$Version-windows-x86_64.zip"
$BaseUrl = "https://github.com/$Repo/releases/download/$Version"
$Tmp = Join-Path $env:TEMP ("cage-install-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $Tmp | Out-Null

function Fetch([string]$Url, [string]$OutFile) {
  # 匿名直链：不带任何 token，不经过 Actions artifacts
  Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing
}

try {
  Write-Host "下载 $Asset ..."
  Fetch "$BaseUrl/$Asset" (Join-Path $Tmp $Asset)
  Fetch "$BaseUrl/SHA256SUMS" (Join-Path $Tmp "SHA256SUMS")

  # SHA256SUMS 行格式（sha256sum --tag）：SHA256 (asset) = <hex>
  $SumsPath = Join-Path $Tmp "SHA256SUMS"
  $Expected = $null
  foreach ($line in Get-Content $SumsPath) {
    if ($line -match "^SHA256 \($Asset\) = ([0-9a-fA-F]{64})$") {
      $Expected = $Matches[1].ToLowerInvariant()
      break
    }
  }
  if (-not $Expected) {
    Write-Error "install.ps1: SHA256SUMS 中找不到 $Asset（该 Release 缺此平台资产或校验单不完整）"
    exit 1
  }
  $Actual = (Get-FileHash (Join-Path $Tmp $Asset) -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($Actual -ne $Expected) {
    Write-Error "install.ps1: SHA256 校验失败`n  期望: $Expected`n  实际: $Actual"
    exit 1
  }
  Write-Host "SHA256 校验通过"

  Expand-Archive -Path (Join-Path $Tmp $Asset) -DestinationPath $Tmp -Force
  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
  Move-Item (Join-Path $Tmp $ExeName) $ExePath -Force

  # 用户级 PATH（无需管理员）；已含则不重复追加
  $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
  if (($UserPath -split ";") -notcontains $InstallDir) {
    $NewPath = if ($UserPath) { "$UserPath;$InstallDir" } else { $InstallDir }
    [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    Write-Host "已把 $InstallDir 加入用户级 PATH（重开终端生效）"
  }

  Write-Host ""
  Write-Host "安装完成：$ExePath"
  & $ExePath --version
} finally {
  Remove-Item $Tmp -Recurse -Force -ErrorAction SilentlyContinue
}
