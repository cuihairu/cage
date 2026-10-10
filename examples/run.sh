#!/usr/bin/env bash
# Cage 完整示例一键跑通：覆盖大多数场景。
#   check → L7 gamerule → 分环境 --env → build(client/server) → 增量 →
#   snapshot → gen → diff → migrate dry-run → registry publish →
#   远程分发全链路（push 直推 + presigned 直推 + 消费方远端 resolve 构建）→
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

step "[1/14] cage check examples/game-config   —— L0-L6 全量校验（好数据应通过）"
echo "\$ cage check examples/game-config"
"$CAGE_BIN" check "$EX"

step "[2/14] cage check --level gamerule examples/game-config   —— L7 业务规则（好数据应通过）"
echo "\$ cage check examples/game-config --level gamerule"
"$CAGE_BIN" check "$EX" --level gamerule

step "[3/14] 分环境验证 —— --env dev/prod 放行、未知环境 exit 2"
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

step "[4/14] cage build --profile client   —— 验证 + 生成全部 13 个 target（5 张表）"
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

step "[5/14] cage build --profile client --incremental   —— 指纹一致 → 跳过重建"
echo "\$ cage build examples/game-config --profile client --incremental"
INC_OUT="$("$CAGE_BIN" build "$EX" --profile client --incremental)"
echo "$INC_OUT"
case "$INC_OUT" in *"up to date"*) ;; *) echo "FAIL: 增量期望 up to date" >&2; exit 1 ;; esac

step "[6/14] cage snapshot --profile client (+ --env prod) + --verify   —— 基线/环境化快照打包与回验"
echo "\$ cage snapshot examples/game-config --profile client"
"$CAGE_BIN" snapshot "$EX" --profile client
SNAP_DIR="$(ls -d "$EX"/build/snapshot/client-*/ | head -1)"
[ -n "$SNAP_DIR" ] || { echo "FAIL: 未生成快照目录" >&2; exit 1; }
echo "\$ cage snapshot <dir> --verify"
"$CAGE_BIN" snapshot "$SNAP_DIR" --verify
echo "\$ cage snapshot examples/game-config --profile client --env prod   —— 环境化出包（目录名带 env 段）"
"$CAGE_BIN" snapshot "$EX" --profile client --env prod
PROD_SNAP_DIR="$(ls -d "$EX"/build/snapshot/client-prod-*/ | head -1)"
[ -n "$PROD_SNAP_DIR" ] || { echo "FAIL: 未生成环境化快照目录" >&2; exit 1; }
echo "\$ cage snapshot <prod-dir> --verify"
"$CAGE_BIN" snapshot "$PROD_SNAP_DIR" --verify

step "[7/14] cage gen --profile client   —— 只生成代码绑定与 JSON Schema（含 proto 定义）"
echo "\$ cage gen examples/game-config --profile client"
"$CAGE_BIN" gen "$EX" --profile client

step "[8/14] cage diff build build   —— 同一构建前后比对（确定性：全不变）"
echo "\$ cage diff examples/game-config/build examples/game-config/build"
DIFF_OUT="$("$CAGE_BIN" diff "$EX/build" "$EX/build")"
echo "$DIFF_OUT"
case "$DIFF_OUT" in *"0 changed"*) ;; *) echo "FAIL: diff 期望 0 changed" >&2; exit 1 ;; esac

step "[9/14] cage build --profile server   —— 服务端视图（server-only 字段 internal_note 可见，client 视图剔除）"
echo "\$ cage build examples/game-config --profile server"
"$CAGE_BIN" build "$EX" --profile server
# 字段级 targets 演示：internal_note 仅在 server 视图出现
grep -q "internal_note" "$EX/build/server/json/Character.json" \
  || { echo "FAIL: server 视图应含 internal_note" >&2; exit 1; }
if grep -q "internal_note" "$EX/build/client/json/Character.json"; then
  echo "FAIL: client 视图不应含 internal_note" >&2; exit 1
fi

step "[10/14] cage migrate --to 0.2.0   —— 声明式迁移 dry-run（不写盘）"
echo "\$ cage migrate examples/game-config --to 0.2.0"
MIG_OUT="$("$CAGE_BIN" migrate "$EX" --to 0.2.0 2>&1)"
echo "$MIG_OUT"
case "$MIG_OUT" in *"dry run, nothing written"*) ;; *) echo "FAIL: migrate 期望 dry run" >&2; exit 1 ;; esac
case "$MIG_OUT" in *"rename_field"*) ;; *) echo "FAIL: migrate 期望 rename_field 步骤" >&2; exit 1 ;; esac

step "[11/14] cage registry publish + list   —— 版本化自校验资产发布到本地仓库（含环境化出包与 E1801）"
REG_DIR="$(mktemp -d)"
echo "\$ cage registry publish examples/game-config --registry $REG_DIR"
"$CAGE_BIN" registry publish "$EX" --registry "$REG_DIR"
echo "\$ cage registry publish examples/game-config --env prod --registry $REG_DIR   # 同版本换环境 → E1801（一版本一包）"
set +e
ENV_PUB_OUT="$("$CAGE_BIN" registry publish "$EX" --env prod --registry "$REG_DIR" 2>&1)"
ENV_PUB_CODE=$?
set -e
echo "$ENV_PUB_OUT"
[ "$ENV_PUB_CODE" -eq 1 ] || { echo "FAIL: 同版本换环境期望 E1801 退出码 1，实际 ${ENV_PUB_CODE}" >&2; exit 1; }
case "$ENV_PUB_OUT" in *E1801*) ;; *) echo "FAIL: 输出未点名 E1801" >&2; exit 1 ;; esac
echo "\$ cage registry publish examples/game-config --env prod --registry $REG_DIR --version 0.1.0-prod"
"$CAGE_BIN" registry publish "$EX" --env prod --registry "$REG_DIR" --version 0.1.0-prod
echo "\$ cage registry list --registry $REG_DIR"
"$CAGE_BIN" registry list --registry "$REG_DIR"
rm -rf "$REG_DIR"

step "[12/14] 远程分发全链路   —— push 直推 + presigned 直推 + 消费方远端 resolve 构建"
# 本地源：示例工程自己的 [registry].path（cage.toml 已声明 reg/）
echo "\$ cage registry publish examples/game-config   # 发布进示例本地注册表（push 源）"
"$CAGE_BIN" registry publish "$EX"
# 静态托管替身：python3 起两个收 PUT 的本地对象库（CI ubuntu 自带 python3）
SRV_DIR="$(mktemp -d)"
cat > "$SRV_DIR/server.py" << 'PYEOF'
import http.server, os, sys
ROOT = sys.argv[1]
class H(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw): super().__init__(*a, directory=ROOT, **kw)
    def do_PUT(self):
        n = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(n)
        p = os.path.join(ROOT, self.path.lstrip('/'))
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, 'wb') as f: f.write(body)
        self.send_response(200); self.send_header('Content-Length','0'); self.end_headers()
    def log_message(self, *a): pass
srv = http.server.ThreadingHTTPServer(('127.0.0.1', 0), H)
print(srv.server_address[1], flush=True)
srv.serve_forever()
PYEOF
mkdir -p "$SRV_DIR/storeA" "$SRV_DIR/storeB"
nohup python3 "$SRV_DIR/server.py" "$SRV_DIR/storeA" > "$SRV_DIR/portA" 2>&1 &
SRV_PID_A=$!
nohup python3 "$SRV_DIR/server.py" "$SRV_DIR/storeB" > "$SRV_DIR/portB" 2>&1 &
SRV_PID_B=$!
trap 'kill $SRV_PID_A $SRV_PID_B 2>/dev/null || true; rm -rf "$SRV_DIR" || true' EXIT
for i in $(seq 1 50); do [ -s "$SRV_DIR/portA" ] && [ -s "$SRV_DIR/portB" ] && break; sleep 0.1; done
PORT_A="$(cat "$SRV_DIR/portA")"
PORT_B="$(cat "$SRV_DIR/portB")"
echo "(本地对象库 A=http://127.0.0.1:$PORT_A 直推 / B=http://127.0.0.1:$PORT_B presigned)"

# ① 直推：匿名探针 + 逐文件 PUT + index 合并收尾
echo "\$ cage registry push examples/game-config --registry http://127.0.0.1:$PORT_A"
"$CAGE_BIN" registry push "$EX" --registry "http://127.0.0.1:$PORT_A"
# 重推同字节 → 幂等零 PUT
PUSH_OUT="$("$CAGE_BIN" registry push "$EX" --registry "http://127.0.0.1:$PORT_A" 2>&1)"
echo "$PUSH_OUT"
case "$PUSH_OUT" in *"identical, no-op"*) ;; *) echo "FAIL: 重推期望 identical no-op" >&2; exit 1 ;; esac

# ② presigned 直推：CI 向对象存储要的 URL 表（uploads 每文件一条 + index GET/PUT 对）
PRESIGN_MAP="$SRV_DIR/presign.json"
ENTRY_VER="$(ls -d "$EX/reg/rpg-demo"/*/ | sed 's:.*/\([^/]*\)/:\1:' | sort -V | tail -1)"
python3 - "$EX/reg" "$PRESIGN_MAP" "$PORT_B" "$ENTRY_VER" << 'PYEOF'
import json, os, sys
reg_root, out_path, port, version = sys.argv[1:5]
uploads = {}
for dp, _, fs in os.walk(os.path.join(reg_root, "rpg-demo", version)):
    for f in fs:
        full = os.path.join(dp, f)
        rel = os.path.relpath(full, reg_root)
        uploads[rel] = f"http://127.0.0.1:{port}/{rel}"
json.dump({"uploads": uploads, "index": {
    "get": f"http://127.0.0.1:{port}/rpg-demo/index.json",
    "put": f"http://127.0.0.1:{port}/rpg-demo/index.json"}}, open(out_path, "w"))
PYEOF
echo "\$ cage registry push examples/game-config --presign-map $PRESIGN_MAP"
"$CAGE_BIN" registry push "$EX" --presign-map "$PRESIGN_MAP"

# ③ 消费方接入：两个工程分别直连 A（直推字节）/ B（presigned 字节）远端根构建
CONS_DIR="$(mktemp -d)"
for pair in "consA:$PORT_A" "consB:$PORT_B"; do
  name="${pair%%:*}"; port="${pair##*:}"
  mkdir -p "$CONS_DIR/$name"
  cat > "$CONS_DIR/$name/cage.toml" << EOF
output_dir = "build"
schema_path = "registry:rpg-demo"

[project]
name = "$name"
version = "0.1.0"

[source_roots]
main = "registry:rpg-demo@$ENTRY_VER"

[registry]
path = "http://127.0.0.1:$port"

[profiles.client]
name = "client"

[[profiles.client.targets]]
format = "json"
output_dir = "build/client/json"
file_template = "{table}.json"
EOF
  echo "\$ cage build $CONS_DIR/$name --profile client   # 消费方直连远端根构建（$name ← $port）"
  "$CAGE_BIN" build "$CONS_DIR/$name" --profile client
done
# 消费方产物与发布方逐字节一致（远端条目 = 本地构建的同一份字节）
for t in Item Character Quest Shop Stage; do
  A="$(sha256sum "$EX/build/client/json/$t.json" | cut -d' ' -f1)"
  B="$(sha256sum "$CONS_DIR/consA/build/client/json/$t.json" | cut -d' ' -f1)"
  C="$(sha256sum "$CONS_DIR/consB/build/client/json/$t.json" | cut -d' ' -f1)"
  if [ "$A" != "$B" ] || [ "$A" != "$C" ]; then
    echo "FAIL: $t 消费方产物与发布方不一致" >&2; exit 1
  fi
done
echo "消费方产物与发布方逐字节一致（5 张表 × 2 条分发路）"

kill $SRV_PID_A $SRV_PID_B 2>/dev/null
rm -rf "$SRV_DIR" "$CONS_DIR" "$EX/reg"

step "[13/14] 坏数据诊断   —— L6 E1501 违反 hp<=attack + E1601 越过 power curve + --env prod E1001 缺昵称"
echo "\$ cage check examples/game-config/bad   # 期望 E1501（hp > attack，L6 语义规则）+ 退出码 1"
set +e
L6_OUT="$("$CAGE_BIN" check "$EX/bad" 2>&1)"
L6_CODE=$?
set -e
echo "$L6_OUT"
[ "$L6_CODE" -eq 1 ] || { echo "FAIL: L6 坏数据期望退出码 1，实际 ${L6_CODE}" >&2; exit 1; }
case "$L6_OUT" in *E1501*) ;; *) echo "FAIL: 输出未包含 E1501" >&2; exit 1 ;; esac
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

step "[14/14] cage inspect examples/game-config   —— 查看表结构"
echo "\$ cage inspect examples/game-config"
"$CAGE_BIN" inspect "$EX"

printf '\n全部通过。\n产物目录：examples/game-config/build/{client,server}/{json,csv,msgpack,proto,cs,py,lua,ts,js,cpp,go,java,jsonschema}\n快照目录：examples/game-config/build/snapshot/\n'
