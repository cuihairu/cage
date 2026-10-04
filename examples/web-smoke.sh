#!/usr/bin/env bash
# cage web 端到端冒烟（第三阶段 W4）：
#   编辑 → 校验 → 保存 → cage build 复现 → 目录 schema 409 拒写。
# 演示对象是 examples/web-demo/（单文件 schema），全部写操作发生在拷贝上，
# 脚本本身不脏仓库；真实命令逐条打印，任一步失败立即退出。
# 用法：bash examples/web-smoke.sh        （默认 cargo build 后取 target/debug/cage）
#       CAGE_BIN=/path/to/cage bash examples/web-smoke.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO="$ROOT/examples/web-demo"
MULTI="$ROOT/examples/game-config"
CAGE_BIN="${CAGE_BIN:-}"

if [ -z "$CAGE_BIN" ]; then
  echo "==> 构建 cage 可执行文件"
  cargo build -q --locked --manifest-path "$ROOT/Cargo.toml" --bin cage
  CAGE_BIN="$ROOT/target/debug/cage"
fi

step() { printf '\n============================================================\n==> %s\n============================================================\n' "$*"; }

# 动态端口 + 就绪轮询（最多 10s），避免固定端口冲突
pick_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

wait_ready() {
  python3 - "$1" <<'PY'
import socket, sys, time
port = int(sys.argv[1])
for _ in range(200):
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.5):
            sys.exit(0)
    except OSError:
        time.sleep(0.05)
sys.exit(1)
PY
}

# 编辑器会话：GET 文档 → 加 stack_size 字段 → 校验 → 保存；全部断言在 Python 内
editor_session() {
  python3 - "$1" "$2" <<'PY'
import json, sys, urllib.request, urllib.error

port, work = sys.argv[1], sys.argv[2]
base = f"http://127.0.0.1:{port}"

def req(method, path, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    r = urllib.request.Request(base + path, data=data, method=method,
                               headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(r, timeout=5) as resp:
            return resp.status, json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read().decode())

def assert_true(cond, msg):
    if not cond:
        sys.exit(f"ASSERT FAIL: {msg}")

# 1) 装载文档
status, doc = req("GET", "/api/schema")
assert_true(status == 200 and doc["ok"], f"GET /api/schema: {status} {doc}")
assert_true("Item" in doc["schema"]["tables"], "merged schema must contain Item")
assert_true(doc["project"] == "web-demo", f"project: {doc.get('project')}")

# 2) 编辑：加可选字段 stack_size（UInt8 + min 1 + 默认 1），保持文档 canonical
fields = doc["schema"]["tables"]["Item"]["fields"]
fields["stack_size"] = {
    "name": "stack_size",
    "type": {"kind": "UInt8"},
    "description": None,
    "default": 1,
    "min": 1,
}

# 3) 校验：编辑态 E1701 + L1 应通过
status, v = req("POST", "/api/validate", doc["schema"])
assert_true(status == 200, f"validate status: {status}")
assert_true(v["ok"], f"validate must pass, got: {v}")
assert_true(v["diagnostics"] == [], f"no diagnostics expected: {v}")

# 4) 保存：canonical YAML 写回 schema.yaml
status, s = req("POST", "/api/schema", doc["schema"])
assert_true(status == 200 and s["ok"], f"save: {status} {s}")
assert_true(s["save_target"].endswith("schema.yaml"), f"target: {s.get('save_target')}")
assert_true(s["bytes"] > 0, "saved bytes > 0")
print(f"editor session ok: validate passed, saved {s['bytes']} bytes to {s['save_target']}")
PY
}

# 目录 schema 409：多文件工程（schemas/）编辑器拒写
dir_refuse() {
  python3 - "$1" <<'PY'
import json, sys, urllib.request, urllib.error

port = sys.argv[1]
body = json.dumps({"tables": {}, "enums": {}, "metadata": None}).encode()
url = f"http://127.0.0.1:{port}/api/schema"
try:
    with urllib.request.urlopen(urllib.request.Request(url, data=body, method="POST",
                                                       headers={"Content-Type": "application/json"}), timeout=5) as resp:
        sys.exit(f"ASSERT FAIL: expected 409, got {resp.status}")
except urllib.error.HTTPError as e:
    assert e.code == 409, f"expected 409, got {e.code}"
    payload = json.loads(e.read().decode())
    assert "directory" in payload["error"], f"error must mention directory: {payload}"
    print(f"directory schema refused ok: 409 {payload['error'][:60]}...")
PY
}

hash_tree() {
  (cd "$1" && find . -type f | sort | xargs sha256sum)
}

WORK="$(mktemp -d)"
trap 'pkill -P $$ 2>/dev/null || true; rm -rf "$WORK"' EXIT
cp -r "$DEMO/." "$WORK/demo"
MULTI_WORK="$WORK/multi"
cp -r "$MULTI/." "$MULTI_WORK"

step "[1/6] cage check examples/web-demo   —— 基线（未编辑 schema 应通过）"
echo "\$ $CAGE_BIN check $WORK/demo"
"$CAGE_BIN" check "$WORK/demo"

step "[2/6] 启动 cage web，读编辑器文档并编辑 → 校验 → 保存（加 stack_size 字段）"
PORT="$(pick_port)"
echo "\$ $CAGE_BIN web $WORK/demo --port $PORT &"
"$CAGE_BIN" web "$WORK/demo" --port "$PORT" &
WEB_PID=$!
wait_ready "$PORT"
editor_session "$PORT" "$WORK/demo"
echo "\$ kill \$WEB_PID"
kill "$WEB_PID"
wait "$WEB_PID" 2>/dev/null || true

step "[3/6] 保存后的 schema.yaml 是 canonical YAML，且包含新字段"
grep -q "stack_size" "$WORK/demo/schema.yaml" || { echo "stack_size 未写入 schema.yaml"; exit 1; }
grep -q "min: 1" "$WORK/demo/schema.yaml" || { echo "min: 1 未写入（UInt8 约束）"; exit 1; }
echo "==> schema.yaml 已含 stack_size（UInt8, min 1, default 1）"

step "[4/6] cage check + cage build   —— 编辑后 schema 全链路可复现"
echo "\$ $CAGE_BIN check $WORK/demo && $CAGE_BIN build $WORK/demo --profile client"
"$CAGE_BIN" check "$WORK/demo"
"$CAGE_BIN" build "$WORK/demo" --profile client

step "[5/6] 确定性复现：同一 canonical schema 重建两次，产物树逐字节一致"
hash_tree "$WORK/demo/build" > "$WORK/h1.txt"
echo "\$ rm -rf build && $CAGE_BIN build ...   # 第二次构建"
rm -rf "$WORK/demo/build"
"$CAGE_BIN" build "$WORK/demo" --profile client >/dev/null
hash_tree "$WORK/demo/build" > "$WORK/h2.txt"
cmp "$WORK/h1.txt" "$WORK/h2.txt" || { echo "两次构建产物哈希不一致"; exit 1; }
echo "==> 两次构建 $(wc -l < "$WORK/h1.txt") 个文件哈希逐字节一致"

step "[6/6] 多文件 schema（schemas/ 目录）→ POST /api/schema 应得 409 拒写"
echo "\$ $CAGE_BIN web $WORK/multi --port $PORT &   # 另一个服务"
PORT2="$(pick_port)"
"$CAGE_BIN" web "$MULTI_WORK" --port "$PORT2" &
MULTI_PID=$!
wait_ready "$PORT2"
dir_refuse "$PORT2"
kill "$MULTI_PID"
wait "$MULTI_PID" 2>/dev/null || true

echo
echo "✔ web 冒烟全部通过：编辑 → 校验 → 保存 → build 复现（确定性字节）+ 目录 409"