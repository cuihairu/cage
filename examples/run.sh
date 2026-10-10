#!/usr/bin/env bash
# Cage 完整示例一键跑通：覆盖大多数场景。
#   check → L7 gamerule → 分环境 --env → build(client/server) → 增量 →
#   snapshot → gen → diff → migrate dry-run → registry publish →
#   坏数据诊断 → inspect
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

step "[1/13] cage check examples/game-config   —— L0-L6 全量校验（好数据应通过）"
echo "\$ cage check examples/game-config"
"$CAGE_BIN" check "$EX"

step "[2/13] cage check --level gamerule examples/game-config   —— L7 业务规则（好数据应通过）"
echo "\$ cage check examples/game-config --level gamerule"
"$CAGE_BIN" check "$EX" --level gamerule

step "[3/13] 分环境验证 —— --env dev/prod 放行、未知环境 exit 2"
echo "\$ cage check examples/game-config --env dev"
"$CAGE_BIN" check "$EX" --env dev
echo "\$ cage check examples/game-config --env prod"
"$CAGE_BIN" check "$EX" --env prod
echo "\$ cage check examples/game-config --env staging   # 期望退出码 2"
set +e
ENV_OUT="$("$CAGE_BIN" check "$EX" --env staging 2>&1)"
ENV_CODE=$?
set -e
echo "$ENV_OUT"
[ "$ENV_CODE" -eq 2 ] || { echo "FAIL: 未知环境期望退出码 2，实际 ${ENV_CODE}" >&2; exit 1; }
case "$ENV_OUT" in *"unknown environment 'staging'"*) ;; *) echo "FAIL: 输出未点名 unknown environment" >&2; exit 1 ;; esac

step "[4/13] cage build --profile client   —— 验证 + 生成全部 12 个 target（5 张表）"
echo "\$ cage build examples/game-config --profile client"
"$CAGE_BIN" build "$EX" --profile client
# 新源产物抽查：Excel 源 Quest.json / MsgPack 源 Shop.json + 新 target 的 proto/msgpack
for f in json/Quest.json json/Shop.json msgpack/Shop.msgpack msgpack/Quest.msgpack \
         proto/Quest.proto proto/Shop.proto proto/cage_enums.proto; do
  test -s "$EX/build/client/$f" || { echo "FAIL: 缺产物 $f" >&2; exit 1; }
done
# 确定性抽查：全量重建后两格式逐字节一致
B1="$(sha256sum "$EX/build/client/msgpack/Item.msgpack" | cut -d' ' -f1)"
B2="$(sha256sum "$EX/build/client/proto/Item.proto" | cut -d' ' -f1)"
"$CAGE_BIN" build "$EX" --profile client > /dev/null
A1="$(sha256sum "$EX/build/client/msgpack/Item.msgpack" | cut -d' ' -f1)"
A2="$(sha256sum "$EX/build/client/proto/Item.proto" | cut -d' ' -f1)"
if [ "$B1" != "$A1" ] || [ "$B2" != "$A2" ]; then
  echo "FAIL: 重建后产物字节变化（期望确定性）" >&2; exit 1
fi

step "[5/13] cage build --profile client --incremental   —— 指纹一致 → 跳过重建"
echo "\$ cage build examples/game-config --profile client --incremental"
INC_OUT="$("$CAGE_BIN" build "$EX" --profile client --incremental)"
echo "$INC_OUT"
case "$INC_OUT" in *"up to date"*) ;; *) echo "FAIL: 增量期望 up to date" >&2; exit 1 ;; esac

step "[6/13] cage snapshot --profile client + --verify   —— 自校验快照打包与回验"
echo "\$ cage snapshot examples/game-config --profile client"
"$CAGE_BIN" snapshot "$EX" --profile client
SNAP_DIR="$(ls -d "$EX"/build/snapshot/client-*/ | head -1)"
[ -n "$SNAP_DIR" ] || { echo "FAIL: 未生成快照目录" >&2; exit 1; }
echo "\$ cage snapshot <dir> --verify"
"$CAGE_BIN" snapshot "$SNAP_DIR" --verify

step "[7/13] cage gen --profile client   —— 只生成 9 种代码绑定（含 proto 定义）"
echo "\$ cage gen examples/game-config --profile client"
"$CAGE_BIN" gen "$EX" --profile client

step "[8/13] cage diff build build   —— 同一构建前后比对（确定性：全不变）"
echo "\$ cage diff examples/game-config/build examples/game-config/build"
DIFF_OUT="$("$CAGE_BIN" diff "$EX/build" "$EX/build")"
echo "$DIFF_OUT"
case "$DIFF_OUT" in *"0 changed"*) ;; *) echo "FAIL: diff 期望 0 changed" >&2; exit 1 ;; esac

step "[9/13] cage build --profile server   —— 服务端视图（server-only 字段 internal_note 可见，client 视图剔除）"
echo "\$ cage build examples/game-config --profile server"
"$CAGE_BIN" build "$EX" --profile server
# 字段级 targets 演示：internal_note 仅在 server 视图出现
grep -q "internal_note" "$EX/build/server/json/Character.json" \
  || { echo "FAIL: server 视图应含 internal_note" >&2; exit 1; }
if grep -q "internal_note" "$EX/build/client/json/Character.json"; then
  echo "FAIL: client 视图不应含 internal_note" >&2; exit 1
fi

step "[10/13] cage migrate --to 0.2.0   —— 声明式迁移 dry-run（不写盘）"
echo "\$ cage migrate examples/game-config --to 0.2.0"
MIG_OUT="$("$CAGE_BIN" migrate "$EX" --to 0.2.0 2>&1)"
echo "$MIG_OUT"
case "$MIG_OUT" in *"dry run, nothing written"*) ;; *) echo "FAIL: migrate 期望 dry run" >&2; exit 1 ;; esac
case "$MIG_OUT" in *"rename_field"*) ;; *) echo "FAIL: migrate 期望 rename_field 步骤" >&2; exit 1 ;; esac

step "[11/13] cage registry publish + list   —— 版本化自校验资产发布到本地仓库"
REG_DIR="$(mktemp -d)"
echo "\$ cage registry publish examples/game-config --registry $REG_DIR"
"$CAGE_BIN" registry publish "$EX" --registry "$REG_DIR"
echo "\$ cage registry list --registry $REG_DIR"
"$CAGE_BIN" registry list --registry "$REG_DIR"
rm -rf "$REG_DIR"

step "[12/13] 坏数据诊断   —— E1601 越过 power curve + --env prod E1001 缺昵称"
echo "\$ cage check examples/game-config/bad --level gamerule   # 期望 E1601 + 退出码 1"
set +e
BAD_OUT="$("$CAGE_BIN" check "$EX/bad" --level gamerule 2>&1)"
BAD_CODE=$?
set -e
echo "$BAD_OUT"
[ "$BAD_CODE" -eq 1 ] || { echo "FAIL: 坏数据期望退出码 1，实际 ${BAD_CODE}" >&2; exit 1; }
case "$BAD_OUT" in *E1601*) ;; *) echo "FAIL: 输出未包含 E1601" >&2; exit 1 ;; esac
echo "\$ cage check examples/game-config/bad --env prod   # prod 收紧 nickname 必填 → E1001"
set +e
PROD_OUT="$("$CAGE_BIN" check "$EX/bad" --env prod 2>&1)"
PROD_CODE=$?
set -e
echo "$PROD_OUT"
[ "$PROD_CODE" -eq 1 ] || { echo "FAIL: prod 坏数据期望退出码 1，实际 ${PROD_CODE}" >&2; exit 1; }
case "$PROD_OUT" in *E1001*) ;; *) echo "FAIL: 输出未包含 E1001" >&2; exit 1 ;; esac

step "[13/13] cage inspect examples/game-config   —— 查看表结构"
echo "\$ cage inspect examples/game-config"
"$CAGE_BIN" inspect "$EX"

printf '\n全部通过。\n产物目录：examples/game-config/build/{client,server}/{json,csv,msgpack,proto,cs,py,lua,ts,js,cpp,go,java}\n快照目录：examples/game-config/build/snapshot/\n'
