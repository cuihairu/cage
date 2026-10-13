#!/bin/sh
# Cage 一键安装（Linux / macOS）：从滚动 nightly Release 匿名直链下载
# 二进制（不走 Actions artifacts、不带任何 token），校验 SHA256 后装入
# ~/.local/bin（可用 CAGE_INSTALL_DIR 覆盖）。Windows 用 install.ps1。
#
# 用法：
#   curl -fsSL https://raw.githubusercontent.com/cuihairu/cage/main/scripts/install.sh | sh
#   ./install.sh                # 装最新 nightly（默认把安装目录加入 PATH）
#   ./install.sh --no-path      # 装完不写 PATH，只打印手动加入方法
#   ./install.sh --path         # 兼容旧写法（写 PATH 现在是默认行为）
#   ./install.sh v0.1.0         # 装指定 Release tag（需该 tag 的 Release 带对应平台资产）
#   ./install.sh --uninstall    # 卸载
#
# 依赖：curl 或 wget 其一；sha256sum 或 shasum 其一；tar + gzip。

set -eu

REPO="${CAGE_REPO:-cuihairu/cage}"
TAG=""
INSTALL_DIR="${CAGE_INSTALL_DIR:-$HOME/.local/bin}"
AUTO_PATH=1
UNINSTALL=0

# 参数整理：第一个非 flag 参数是 Release tag（默认 nightly）
for arg in "$@"; do
  case "$arg" in
    --uninstall) UNINSTALL=1 ;;
    --path) AUTO_PATH=1 ;;   # 兼容旧写法：写 PATH 现在是默认行为
    --no-path) AUTO_PATH=0 ;;
    -h|--help)
      sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    --*) echo "install.sh: unknown option: $arg" >&2; exit 1 ;;
    *)
      if [ -n "$TAG" ]; then
        echo "install.sh: multiple versions given ('$TAG' and '$arg')" >&2
        exit 1
      fi
      TAG="$arg"
      ;;
  esac
done
[ -n "$TAG" ] || TAG=nightly

BIN_NAME=cage
INSTALL_DIR="${INSTALL_DIR%/}"

uninstall() {
  if [ -f "$INSTALL_DIR/$BIN_NAME" ]; then
    rm -f "$INSTALL_DIR/$BIN_NAME"
    echo "removed $INSTALL_DIR/$BIN_NAME"
  else
    echo "install.sh: $INSTALL_DIR/$BIN_NAME not found (nothing to uninstall)" >&2
    exit 1
  fi
  echo '若安装目录已不在使用，可从 shell profile 移除对应的 PATH 行。'
}

if [ "$UNINSTALL" = 1 ]; then
  uninstall
  exit 0
fi

case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=macos ;;
  MINGW*|MSYS*|CYGWIN*)
    echo 'install.sh: Windows 请使用 install.ps1（PowerShell）' >&2
    exit 1
    ;;
  *) echo "install.sh: unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  aarch64|arm64) arch=aarch64 ;;
  *) echo "install.sh: unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

ASSET="$BIN_NAME-$TAG-$os-$arch.tar.gz"
BASE_URL="https://github.com/$REPO/releases/download/$TAG"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fetch() {
  # fetch <url> <outfile>：curl 优先，wget 兜底，匿名直链
  if command -v curl >/dev/null 2>&1; then
    curl -fSL --retry 3 --proto '=https' -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$1"
  else
    echo 'install.sh: 需要 curl 或 wget 之一（安装后重试）' >&2
    exit 1
  fi
}

hash_of() {
  # hash_of <file>：跨平台取 sha256（GNU sha256sum / BSD shasum / busybox）
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    echo 'install.sh: 需要 sha256sum 或 shasum 之一（校验用）' >&2
    exit 1
  fi
}

echo "下载 $ASSET ..."
fetch "$BASE_URL/$ASSET" "$TMP/$ASSET"
fetch "$BASE_URL/SHA256SUMS" "$TMP/SHA256SUMS"

expected="$(sed -n "s/^SHA256 ($ASSET) = \([0-9a-fA-F]\{64\}\)\$/\1/p" "$TMP/SHA256SUMS")"
if [ -z "$expected" ]; then
  echo "install.sh: SHA256SUMS 中找不到 $ASSET（该 Release 缺此平台资产或校验单不完整）" >&2
  exit 1
fi
actual="$(hash_of "$TMP/$ASSET")"
if [ "$actual" != "$(echo "$expected" | tr 'A-F' 'a-f')" ]; then
  echo "install.sh: SHA256 校验失败" >&2
  echo "  期望: $expected" >&2
  echo "  实际: $actual" >&2
  exit 1
fi
echo "SHA256 校验通过"

tar xzf "$TMP/$ASSET" -C "$TMP"
mkdir -p "$INSTALL_DIR"
mv "$TMP/$BIN_NAME" "$INSTALL_DIR/$BIN_NAME"
chmod +x "$INSTALL_DIR/$BIN_NAME"

# add_path_line <profile>：把安装目录幂等写入 shell profile（跑两次只留一行）。
# 写入前备份 <profile>.bak-<日期>；写失败显式降级说明，不静默跳过。
add_path_line() {
  local line profile bak
  profile="$1"
  line="export PATH=\"$INSTALL_DIR:\$PATH\""
  if [ -f "$profile" ] && grep -qs "PATH=.*$INSTALL_DIR" "$profile"; then
    echo "PATH 配置已存在于 $profile（跳过）"
    return 0
  fi
  if [ -f "$profile" ]; then
    bak="$profile.bak-$(date +%Y%m%d)"
    if ! cp -p "$profile" "$bak" 2>/dev/null; then
      echo "install.sh: 警告：备份 $profile 失败（$bak），继续写入" >&2
    fi
  fi
  if printf '\n# added by cage install.sh\n%s\n' "$line" 2>/dev/null >> "$profile"; then
    echo "已写入 $profile：$line"
    echo "执行 source $profile 或重开终端生效"
  else
    echo "install.sh: 警告：写入 $profile 失败，PATH 未配置。" >&2
    echo "  请手动添加：export PATH=\"$INSTALL_DIR:\$PATH\"" >&2
  fi
}

if [ "$AUTO_PATH" = 1 ]; then
  case "${SHELL:-}" in
    *zsh) profile="$HOME/.zshrc" ;;
    *bash)
      # bash 登录壳（ssh / bash -l / su）不读 .bashrc，只有交互非登录壳读；
      # 锚定登录 profile 让两类会话都能拿到 PATH。
      if [ -f "$HOME/.bash_profile" ]; then
        profile="$HOME/.bash_profile"
      else
        profile="$HOME/.profile"
      fi
      ;;
    *) profile="$HOME/.profile" ;;
  esac
  add_path_line "$profile"
else
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
      echo
      echo "注意：未把 $INSTALL_DIR 加入 PATH（--no-path）。手动加入："
      echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
      ;;
  esac
fi

echo
echo "安装完成：$INSTALL_DIR/$BIN_NAME"
"$INSTALL_DIR/$BIN_NAME" --version
