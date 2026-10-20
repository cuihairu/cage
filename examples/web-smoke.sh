#!/usr/bin/env bash
# cage web 端到端冒烟（第三阶段 W4 + U1 状态面 + U2a 产物 + U2b 投影
#   + U3 快照/对比/registry）：编辑 → 校验 → 保存 → cage build 复现
#   → 状态页（新鲜度 + up-to-date + 产物账本 + 快照 + 分层 diff）
#   → 目录 schema 409 拒写 + profile 投影。
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

# 状态页会话（U1）：页面 → /api/status 新鲜度 → /api/build up-to-date
status_session() {
  python3 - "$1" <<'PY'
import json, sys, urllib.parse, urllib.request, urllib.error

port = sys.argv[1]
base = f"http://127.0.0.1:{port}"

def req_raw(method, path, body=None):
    # Read-side callers pass only the path; uploads POST raw file bytes.
    r = urllib.request.Request(base + path, data=body, method=method,
                               headers={"Content-Type": "application/octet-stream"} if body else {})
    try:
        with urllib.request.urlopen(r, timeout=10) as resp:
            raw = resp.read().decode()
            try:
                return resp.status, json.loads(raw)
            except json.JSONDecodeError:
                return resp.status, raw
    except urllib.error.HTTPError as e:
        raw = e.read().decode()
        try:
            return e.code, json.loads(raw)
        except json.JSONDecodeError:
            return e.code, raw

def req(method, path, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    r = urllib.request.Request(base + path, data=data, method=method,
                               headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(r, timeout=10) as resp:
            return resp.status, json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read().decode())

def assert_true(cond, msg):
    if not cond:
        sys.exit(f"ASSERT FAIL: {msg}")

# 1) 页面本体（GET /status 会被重定向到 /status/，urlopen 自动跟随）
status, html = req_raw("GET", "/status/")
assert_true(status == 200 and "项目状态" in html, f"status page: {status}")

# 2) 状态：校验通过 + client 构建新鲜（上一步刚 cage build 过）
status, v = req("GET", "/api/status?profile=client")
assert_true(status == 200 and v["ok"], f"GET /api/status: {status} {v}")
assert_true(v["validation"]["ok"], f"validation must pass: {v['validation']}")
client = v["builds"]["client"]
assert_true(client["fresh"], f"client build must be fresh: {client}")
assert_true(client["last_build_profile"] == "client", f"manifest owner: {client}")
assert_true(len(client["build_id"]) >= 12, f"build_id present: {client}")

# 3) API 构建：同一 build 管线，已是最新 → up_to_date
status, b = req("POST", "/api/build", {"profile": "client"})
assert_true(status == 200 and b["ok"] and b["up_to_date"], f"build: {status} {b}")

# 4) 产物（U2a）：manifest 账本列表 + 验哈希伺服
status, arts = req("GET", "/api/artifacts?profile=client")
assert_true(status == 200 and arts["ok"], f"GET /api/artifacts: {status} {arts}")
assert_true(arts["manifest_profile"] == "client", f"owner: {arts}")
assert_true(len(arts["artifacts"]) >= 1, f"artifacts: {arts}")
rel = arts["artifacts"][0]["path"]
status, raw = req_raw("GET", f"/api/artifact?path={urllib.parse.quote(rel)}")
assert_true(status == 200 and len(raw) > 0, f"artifact serve {rel}: {status}")
status, raw = req("GET", "/api/artifact?path=schema.yaml")
assert_true(status == 404, f"non-ledgered path must 404, got {status}")

# 5) 快照（U3）：打包 → 列表自验 → 对比（同输入 = 同字节 → 零差异）
status, pack = req("POST", "/api/snapshot", {"profile": "client"})
assert_true(status == 200 and pack["ok"], f"POST /api/snapshot: {status} {pack}")
snap_dir = pack["snapshot_dir"]
assert_true(snap_dir.startswith("build/snapshot/client-"), f"pack dir: {snap_dir}")
status, snaps = req("GET", "/api/snapshots")
assert_true(status == 200 and snaps["ok"], f"GET /api/snapshots: {status} {snaps}")
assert_true(len(snaps["snapshots"]) == 1, f"snapshots: {snaps}")
assert_true(snaps["snapshots"][0]["verified"], f"pack must self-verify: {snaps}")
status, d = req("GET", f"/api/diff?from=build&to={urllib.parse.quote(snap_dir)}")
assert_true(status == 200 and d["ok"], f"GET /api/diff: {status} {d}")
assert_true(d["files"]["added"] == [] and d["files"]["removed"] == [] and d["files"]["changed"] == [],
            f"identical inputs diff to nothing: {d}")
assert_true(d["roots"]["build_id"]["from"] == d["roots"]["build_id"]["to"], f"roots: {d}")

# 6) registry（U3）：demo 工程未配置 [registry] → 空列表 + note
status, reg = req("GET", "/api/registry")
assert_true(status == 200 and reg["ok"], f"GET /api/registry: {status} {reg}")
assert_true(reg["packages"] == [] and "registry" in reg.get("note", ""), f"registry: {reg}")

# 7) 数据面板（策划界面 P0）：表清单 + 行分页 + 隐藏表 404
status, ts = req("GET", "/api/tables?profile=client")
assert_true(status == 200 and ts["ok"], f"GET /api/tables: {status} {ts}")
names = [t["name"] for t in ts["tables"]]
assert_true(len(names) >= 1, f"tables inventory: {ts}")
status, t = req("GET", f"/api/table?name={urllib.parse.quote(names[0])}")
assert_true(status == 200 and t["ok"], f"GET /api/table: {status} {t}")
assert_true(t["total"] == len(t["rows"]), f"single page holds all rows: {t}")
assert_true(len(t["columns"]) >= 1 and len(t["primary_key"]) >= 1, f"columns/pk: {t}")
status, t = req("GET", "/api/table?name=Ghost")
assert_true(status == 404, f"hidden table must 404, got {status}")

# 8) 上传替换（策划界面 P0）：整文件原子替换 + 校验不过不落盘
target = urllib.parse.quote(names[0])
good = json.dumps({names[0]: [{"id": 1, "name": "uploaded"}]}).encode()
status, t = req_raw("POST", f"/api/upload?name={target}", good)
assert_true(status == 200 and t["ok"], f"upload accepted: {status} {t}")
status, t = req("GET", f"/api/table?name={target}")
assert_true(t["rows"][0]["cells"]["name"] == "uploaded", f"upload visible via panel: {t}")
bad = json.dumps({names[0]: [{"id": "not-an-int", "name": "x"}]}).encode()
status, t = req_raw("POST", f"/api/upload?name={target}", bad)
assert_true(status == 200 and not t["ok"] and t.get("diagnostics"), f"bad upload refused: {status} {t}")
status, t = req("GET", f"/api/table?name={target}")
assert_true(t["rows"][0]["cells"]["name"] == "uploaded", f"original restored after refusal: {t}")

print("status session ok: page served, client fresh, rebuild up-to-date, artifacts served, snapshot packed+verified+diffed, registry browsed, data panel listed")
PY
}

# 目录 schema 409：多文件工程（schemas/）编辑器拒写 + U2b profile 投影
dir_refuse() {
  python3 - "$1" <<'PY'
import json, sys, urllib.request, urllib.error

port = sys.argv[1]
base = f"http://127.0.0.1:{port}"

def req(method, path, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    r = urllib.request.Request(base + path, data=data, method=method,
                               headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(r, timeout=10) as resp:
            return resp.status, json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read().decode())

def assert_true(cond, msg):
    if not cond:
        sys.exit(f"ASSERT FAIL: {msg}")

# 1) 目录 schema 仍是 409 拒写
body = json.dumps({"tables": {}, "enums": {}, "metadata": None})
status, payload = req("POST", "/api/schema", json.loads(body))
assert_true(status == 409 and "directory" in payload["error"],
            f"directory must 409: {status} {payload}")

# 2) U2b 投影：internal_note 是 server-only 字段
status, doc = req("GET", "/api/schema?profile=client")
assert_true(status == 200 and doc["ok"], f"GET /api/schema?profile=client: {status}")
assert_true(doc["projection"]["profile"] == "client", f"projection owner: {doc.get('projection')}")
fields = doc["projection"]["schema"]["tables"]["Character"]["fields"]
assert_true("internal_note" not in fields, "client projection must drop internal_note")
full_fields = doc["schema"]["tables"]["Character"]["fields"]
assert_true("internal_note" in full_fields, "full document must keep internal_note")

status, doc = req("GET", "/api/schema?profile=server")
assert_true(status == 200 and doc["ok"], f"GET /api/schema?profile=server: {status}")
fields = doc["projection"]["schema"]["tables"]["Character"]["fields"]
assert_true("internal_note" in fields, "server projection must keep internal_note")

status, doc = req("GET", "/api/schema?profile=ghost")
assert_true(status == 400 and not doc["ok"], f"unknown profile must 400: {status} {doc}")
print("directory refuse + profile projection ok: 409, client hides internal_note, server keeps it, ghost 400")
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

step "[1/7] cage check examples/web-demo   —— 基线（未编辑 schema 应通过）"
echo "\$ $CAGE_BIN check $WORK/demo"
"$CAGE_BIN" check "$WORK/demo"

step "[2/7] 启动 cage web，读编辑器文档并编辑 → 校验 → 保存（加 stack_size 字段）"
PORT="$(pick_port)"
echo "\$ $CAGE_BIN web $WORK/demo --port $PORT &"
"$CAGE_BIN" web "$WORK/demo" --port "$PORT" &
WEB_PID=$!
wait_ready "$PORT"
editor_session "$PORT" "$WORK/demo"
echo "\$ kill \$WEB_PID"
kill "$WEB_PID"
wait "$WEB_PID" 2>/dev/null || true

step "[3/7] 保存后的 schema.yaml 是 canonical YAML，且包含新字段"
grep -q "stack_size" "$WORK/demo/schema.yaml" || { echo "stack_size 未写入 schema.yaml"; exit 1; }
grep -q "min: 1" "$WORK/demo/schema.yaml" || { echo "min: 1 未写入（UInt8 约束）"; exit 1; }
echo "==> schema.yaml 已含 stack_size（UInt8, min 1, default 1）"

step "[4/7] cage check + cage build   —— 编辑后 schema 全链路可复现"
echo "\$ $CAGE_BIN check $WORK/demo && $CAGE_BIN build $WORK/demo --profile client"
"$CAGE_BIN" check "$WORK/demo"
"$CAGE_BIN" build "$WORK/demo" --profile client

step "[5/7] cage web /status —— 状态页（U1/U2/U3 + 数据面板）：新鲜度 + 产物 + 快照 + 对比 + registry + 表清单"
PORT="$(pick_port)"
echo "\$ $CAGE_BIN web $WORK/demo --port $PORT &"
"$CAGE_BIN" web "$WORK/demo" --port "$PORT" &
WEB_PID=$!
wait_ready "$PORT"
status_session "$PORT"
echo "\$ kill \$WEB_PID"
kill "$WEB_PID"
wait "$WEB_PID" 2>/dev/null || true
# 快照包落在 build/snapshot/ 下，而第 6 步比较的是 build 树的确定性
# 字节——清掉派生包，让下一步面对未打包的纯净构建树。
rm -rf "$WORK/demo/build/snapshot"

step "[6/7] 确定性复现：同一 canonical schema 重建两次，产物树逐字节一致"
# 两次都从全新构建取哈希：第 5 步的上传替换过源数据，第一颗哈希
# 不能复用旧构建树（否则比的是「源变了」，不是确定性）。
rm -rf "$WORK/demo/build"
"$CAGE_BIN" build "$WORK/demo" --profile client >/dev/null
hash_tree "$WORK/demo/build" > "$WORK/h1.txt"
echo "\$ rm -rf build && $CAGE_BIN build ...   # 第二次构建"
rm -rf "$WORK/demo/build"
"$CAGE_BIN" build "$WORK/demo" --profile client >/dev/null
hash_tree "$WORK/demo/build" > "$WORK/h2.txt"
cmp "$WORK/h1.txt" "$WORK/h2.txt" || { echo "两次构建产物哈希不一致"; exit 1; }
echo "==> 两次构建 $(wc -l < "$WORK/h1.txt") 个文件哈希逐字节一致"

step "[7/7] 多文件 schema 409 拒写 + U2b profile 投影（internal_note 仅 server 可见）"
echo "\$ $CAGE_BIN web $WORK/multi --port $PORT &   # 另一个服务"
PORT2="$(pick_port)"
"$CAGE_BIN" web "$MULTI_WORK" --port "$PORT2" &
MULTI_PID=$!
wait_ready "$PORT2"
dir_refuse "$PORT2"
kill "$MULTI_PID"
wait "$MULTI_PID" 2>/dev/null || true

echo
echo
echo "✔ web 冒烟全部通过：编辑 → 校验 → 保存 → build 复现（确定性字节）→ 状态页（新鲜度 + up-to-date + 产物 + 快照 + 分层 diff + registry）→ 目录 409 + profile 投影"