#!/usr/bin/env bash
# Cage 完整示例一键跑通：check → build → gen → 坏数据诊断 → inspect。
# 每步打印真实执行的命令；任一步失败立即退出（set -e），断言写在脚本里。
# 用法：bash examples/run.sh          （默认 cargo build 后用 target/debug/cage）
#       CAGE_BIN=/path/to/cage bash examples/run.sh   （指定现成可执行文件）
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EX="$ROOT/examples/game-config"
CAGE_BIN="${CAGE_BIN:-}"

if [ -z "$CAGE_BIN" ]; then
  echo "==> 构建 cage 可执行文件"
  cargo build -q --locked --manifest-path "$ROOT/Cargo.toml" --bin cage
  CAGE_BIN="$ROOT/target/debug/cage"
fi

step() { printf '\n============================================================\n==> %s\n============================================================\n' "$*"; }

step "[1/5] cage check examples/game-config   —— L0-L6 全量校验（好数据应通过）"
echo "\$ cage check examples/game-config"
"$CAGE_BIN" check "$EX"

step "[2/5] cage check --level gamerule examples/game-config   —— L7 业务规则（好数据应通过）"
echo "\$ cage check examples/game-config --level gamerule"
"$CAGE_BIN" check "$EX" --level gamerule

step "[3/5] cage build --profile client   —— 验证 + 生成全部 10 个 target"
echo "\$ cage build examples/game-config --profile client"
"$CAGE_BIN" build "$EX" --profile client

step "[4/5] cage gen --profile client   —— 只生成 8 种代码绑定（跳过数据 target）"
echo "\$ cage gen examples/game-config --profile client"
"$CAGE_BIN" gen "$EX" --profile client

step "[5/5] 坏数据：attack=500 越过 power curve（level=1 上限 150）→ 期望 E1601 + 退出码 1"
echo "\$ cage check examples/game-config/bad --level gamerule   # 期望退出码 1"
set +e
BAD_OUT="$("$CAGE_BIN" check "$EX/bad" --level gamerule 2>&1)"
BAD_CODE=$?
set -e
echo "$BAD_OUT"
if [ "$BAD_CODE" -ne 1 ]; then
  echo "FAIL: 坏数据期望退出码 1，实际 ${BAD_CODE}" >&2
  exit 1
fi
case "$BAD_OUT" in
  *E1601*) ;;
  *) echo "FAIL: 输出未包含 E1601 诊断" >&2; exit 1 ;;
esac

step "附：cage inspect examples/game-config   —— 查看表结构"
echo "\$ cage inspect examples/game-config"
"$CAGE_BIN" inspect "$EX"

printf '\n全部通过。\n产物目录：examples/game-config/build/client/{json,csv,cs,py,lua,ts,js,cpp,go,java}\n'
