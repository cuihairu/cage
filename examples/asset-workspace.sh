#!/usr/bin/env bash
# Cage 资产体系一键演练（design §50）：examples/asset-workspace/ 全生命周期。
#   盘点 → 基线 → 篡改复核 → 校验 → 双 profile 构建 → 快照 → 注册表发布 →
#   签名导出 bundle + commit sidecar → 灾难恢复演练（干净目录 import +
#   verify + 消费方重建逐字节 diff）。
# 每步打印真实执行的命令；任一步失败立即退出（set -e），断言写在脚本里。
# 用法：bash examples/asset-workspace.sh
#       CAGE_BIN=/path/to/cage bash examples/asset-workspace.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EX="$ROOT/examples/asset-workspace"
CAGE_BIN="${CAGE_BIN:-}"

if [ -z "$CAGE_BIN" ]; then
  echo "==> 构建 cage 可执行文件"
  cargo build -q --locked --manifest-path "$ROOT/Cargo.toml" --bin cage
  CAGE_BIN="$ROOT/target/debug/cage"
fi

step() { printf '\n============================================================\n==> %s\n============================================================\n' "$*"; }

# 演练工作区（bundle / 密钥 / 恢复现场）：脚本结束即清，不留进仓库
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

step "[1/8] cage assets   —— 资产清单：构建规则 / 规则 / 源数据 三类逐文件 blake3"
echo "\$ cage assets examples/asset-workspace"
"$CAGE_BIN" assets "$EX"

step "[2/8] cage assets --write assets.json   —— 基线随仓入库（备份第一层防线）"
echo "\$ cage assets examples/asset-workspace --write assets.json"
"$CAGE_BIN" assets "$EX" --write "$EX/assets.json"

step "[3/8] cage assets --check assets.json   —— 干净树通过；篡改源数据 → exit 1 点名 changed"
echo "\$ cage assets examples/asset-workspace --check assets.json"
"$CAGE_BIN" assets "$EX" --check "$EX/assets.json"
cp "$EX/sources/Item.csv" "$WORK/Item.csv.orig"
echo "5,poison,Consumable,common,1" >> "$EX/sources/Item.csv"
set +e
TAMPER_OUT="$("$CAGE_BIN" assets "$EX" --check "$EX/assets.json" 2>&1)"
TAMPER_CODE=$?
set -e
echo "$TAMPER_OUT"
[ "$TAMPER_CODE" -eq 1 ] || { echo "FAIL: 篡改后期望退出码 1，实际 ${TAMPER_CODE}" >&2; exit 1; }
case "$TAMPER_OUT" in *"changed   sources/Item.csv"*) ;; *) echo "FAIL: 输出未点名 changed sources/Item.csv" >&2; exit 1 ;; esac
cp "$WORK/Item.csv.orig" "$EX/sources/Item.csv"
echo "\$ （还原后复核）cage assets --check assets.json"
"$CAGE_BIN" assets "$EX" --check "$EX/assets.json"

step "[4/8] cage check + build 双 profile   —— 规则资产驱动校验与确定性构建"
echo "\$ cage check examples/asset-workspace"
"$CAGE_BIN" check "$EX"
echo "\$ cage build examples/asset-workspace --profile client"
"$CAGE_BIN" build "$EX" --profile client
echo "\$ cage build examples/asset-workspace --profile server"
"$CAGE_BIN" build "$EX" --profile server
grep -q "internal_note" "$EX/build/server/json/Character.json" \
  || { echo "FAIL: server 视图应含 internal_note" >&2; exit 1; }
if grep -q "internal_note" "$EX/build/client/json/Character.json"; then
  echo "FAIL: client 视图不应含 internal_note" >&2; exit 1
fi

step "[5/8] cage snapshot --profile client + --verify   —— 发布快照（manifest 记住当时用了哪版规则）"
echo "\$ cage snapshot examples/asset-workspace --profile client"
"$CAGE_BIN" snapshot "$EX" --profile client
SNAP_DIR="$(ls -d "$EX"/build/snapshot/client-*/ | head -1)"
[ -n "$SNAP_DIR" ] || { echo "FAIL: 未生成快照目录" >&2; exit 1; }
echo "\$ cage snapshot <dir> --verify"
"$CAGE_BIN" snapshot "$SNAP_DIR" --verify

step "[6/8] cage registry publish / list / verify   —— 版本化入本地注册表"
echo "\$ cage registry publish examples/asset-workspace --profile client"
"$CAGE_BIN" registry publish "$EX" --profile client
echo "\$ cage registry list --registry $EX/reg"
"$CAGE_BIN" registry list --registry "$EX/reg"
echo "\$ cage registry verify --registry $EX/reg"
"$CAGE_BIN" registry verify --registry "$EX/reg"

step "[7/8] keygen + export --sign + commit sidecar   —— 签名分发三件套（bundle + .sig + .commit）"
echo "\$ cage registry keygen -o <seed-file>   # seed 保密，public key 进 --key-env"
KEYGEN_OUT="$("$CAGE_BIN" registry keygen -o "$WORK/signing.key")"
echo "$KEYGEN_OUT"
PUBLIC="$(printf '%s\n' "$KEYGEN_OUT" | sed -n 's/^cage registry keygen: public key: //p')"
[ -n "$PUBLIC" ] || { echo "FAIL: 未解析到 public key" >&2; exit 1; }
export ASSET_DEMO_SIGNING_KEY="$(cat "$WORK/signing.key")"
export ASSET_DEMO_VERIFYING_KEY="$PUBLIC"
BUNDLE="$WORK/asset-demo-0.1.0.tar"
echo "\$ cage registry export asset-demo@0.1.0 -o $BUNDLE --sign --key-env ASSET_DEMO_SIGNING_KEY"
"$CAGE_BIN" registry export "asset-demo@0.1.0" -o "$BUNDLE" --sign --key-env ASSET_DEMO_SIGNING_KEY --registry "$EX/reg"
# commit sidecar：发布流水线封存产线 commit（历史锚点，确定性契约之外，
# 见 examples/asset-workspace/README.md——不进 manifest 的原因）
git -C "$ROOT" rev-parse HEAD > "$BUNDLE.commit" 2>/dev/null \
  || echo "unknown (not a git checkout)" > "$BUNDLE.commit"
echo "commit sidecar: $(cat "$BUNDLE.commit")"

step "[8/8] 灾难恢复演练   —— 干净目录 import（签名+账本双门）→ verify → 消费方重建逐字节 diff"
RESTORE="$WORK/disaster-recovery"
mkdir -p "$RESTORE/reg" "$RESTORE/consumer"
echo "\$ cage registry import $BUNDLE --verify-sig --key-env ASSET_DEMO_VERIFYING_KEY"
"$CAGE_BIN" registry import "$BUNDLE" --verify-sig --key-env ASSET_DEMO_VERIFYING_KEY --registry "$RESTORE/reg"
echo "\$ cage registry verify --registry <恢复出的注册表>"
"$CAGE_BIN" registry verify --registry "$RESTORE/reg"
cat > "$RESTORE/consumer/cage.toml" << EOF
output_dir = "build"
schema_path = "registry:asset-demo"

[project]
name = "asset-demo-consumer"
version = "0.1.0"

[source_roots]
main = "registry:asset-demo@0.1.0"

[registry]
path = "$RESTORE/reg"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
EOF
echo "\$ cage build <消费方工程> --profile client   # 从恢复出的注册表重建"
"$CAGE_BIN" build "$RESTORE/consumer" --profile client
for t in Item Character; do
  A="$(sha256sum "$EX/build/client/json/$t.json" | cut -d' ' -f1)"
  B="$(sha256sum "$RESTORE/consumer/build/client/json/$t.json" | cut -d' ' -f1)"
  [ "$A" = "$B" ] || { echo "FAIL: $t 重建产物与灾前不一致" >&2; exit 1; }
done
echo "恢复演练通过：bundle 签名验证 + 账本复核 + 重建产物与灾前逐字节一致（Item / Character）"

rm -rf "$EX/reg"

printf '\n全部通过。\n基线：examples/asset-workspace/assets.json（随仓入库）\n快照目录：examples/asset-workspace/build/snapshot/\n三件套（bundle + .sig + .commit）在演练工作区，已随脚本清理——真实发布请送冷备。\n'