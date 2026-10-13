//! `cage mcp --stdio` 集成测试：起真 server，走一遍真实 MCP 会话——
//! initialize → tools/list → 多个 tools/call（含一条故意写错的配置），
//! 断言结构化错误带定位（file:line/row:col）与建议。客户端不引依赖：
//! 直接按行读写 newline-delimited JSON-RPC。

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl McpClient {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cage"))
            .arg("mcp")
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cage mcp --stdio");
        let stdin = child.stdin.take().expect("stdin pipe");
        let stdout = child.stdout.take().expect("stdout pipe");
        Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        }
    }

    /// 发一条请求并等待同 id 的响应（跳过 notification 行）。
    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{line}").expect("write request");
        self.stdin.flush().expect("flush request");
        loop {
            let mut buf = String::new();
            let n = self.stdout.read_line(&mut buf).expect("read response");
            assert!(n > 0, "server closed stdout waiting for id {id}");
            let msg: Value = serde_json::from_str(buf.trim()).expect("parse response line");
            if msg.get("id").and_then(Value::as_u64) == Some(id) {
                return msg;
            }
        }
    }

    /// tools/call 的文本结果解成 JSON（工具输出一律 JSON 字符串）。
    fn call_tool(&mut self, id: u64, name: &str, arguments: Value) -> Value {
        let resp = self.request(
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        );
        let result = resp["result"].clone();
        assert!(
            result
                .get("isError")
                .is_none_or(|e| !e.as_bool().unwrap_or(false)),
            "tool {name} transport error: {result}"
        );
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("tool {name} text content: {result}"));
        serde_json::from_str(text).unwrap_or_else(|_| panic!("tool {name} returns JSON: {text}"))
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // 响应都已读完：冲掉缓冲即可。子进程由 kill 收尾（stdio 管道随之
        // 关闭），不用等服务端优雅退出——测试关心的是它答过什么。
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn example_project() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/game-config")
}

/// 故意写错的工程：price 有负值（E1201）与非数字（E1101 类型不符）。
fn broken_project(dir: &Path) -> PathBuf {
    let root = dir.join("broken");
    std::fs::create_dir_all(root.join("schemas")).unwrap();
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::write(
        root.join("cage.toml"),
        "schema_path = \"schemas\"\n[project]\nname = \"broken\"\n[source_roots]\nmain = \"config\"\n[profiles.client]\nname = \"client\"\ntargets = [{ format = \"json\", output_dir = \"build/client/json\" }]\n",
    )
    .unwrap();
    std::fs::write(
        root.join("schemas/item.yaml"),
        "tables:\n  Item:\n    name: Item\n    primary_key: [id]\n    fields:\n      id: { name: id, type: { kind: Int32 }, required: true }\n      price: { name: price, type: { kind: Int32 }, required: true, min: 0 }\nenums: {}\n",
    )
    .unwrap();
    std::fs::write(root.join("config/Item.csv"), "id,price\n1,-5\n2,oops\n").unwrap();
    root
}

#[test]
fn full_session_initialize_list_tools_and_calls() {
    let mut client = McpClient::spawn();

    // initialize：serverInfo 名字是 cage，声明 tools 能力
    let resp = client.request(
        1,
        "initialize",
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "cage-itest", "version": "0"}
        }),
    );
    assert!(
        resp["result"]["capabilities"]["tools"].is_object(),
        "resp: {resp}"
    );
    assert_eq!(resp["result"]["serverInfo"]["name"], json!("cage"));

    // initialized notification（无响应，直接发）
    writeln!(
        client.stdin,
        "{{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}}"
    )
    .unwrap();
    client.stdin.flush().unwrap();

    // tools/list：六个工具全在，带 inputSchema
    let resp = client.request(2, "tools/list", json!({}));
    let tools: Vec<&str> = resp["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    for expected in [
        "check_schema",
        "list_tables",
        "list_fields",
        "list_envs",
        "build_dry_run",
        "explain_config",
    ] {
        assert!(tools.contains(&expected), "tools: {tools:?}");
    }
    assert!(resp["result"]["tools"][0]["inputSchema"].is_object());

    let example = example_project();
    let example_str = example.to_str().unwrap();

    // 真调用 1：check_schema（好工程 + --env dev 语义）
    let v = client.call_tool(
        3,
        "check_schema",
        json!({"path": example_str, "env": "dev"}),
    );
    assert_eq!(v["ok"], json!(true), "check_schema: {v}");
    assert!(v["tables_checked"].as_u64().unwrap() > 0);

    // 真调用 2：list_envs（四套环境差异）
    let v = client.call_tool(4, "list_envs", json!({"path": example_str}));
    let declared: Vec<&str> = v["declared"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d.as_str())
        .collect();
    for env in ["dev", "test", "staging", "prod"] {
        assert!(declared.contains(&env), "declared: {declared:?}");
    }
    assert_eq!(
        v["tables"]["Shop"]["env_overrides"]["prod"]["discount"]["max"],
        json!(0.5)
    );

    // 真调用 3：list_fields（结构与类型）
    let v = client.call_tool(
        5,
        "list_fields",
        json!({"path": example_str, "table": "Item"}),
    );
    assert_eq!(v["tables"][0]["name"], json!("Item"));
    assert!(v["tables"][0]["fields"]
        .as_array()
        .is_some_and(|f| f.len() >= 3));

    // 真调用 4：check_schema（故意写错的配置）——错误码 + file:line 定位 + 建议
    let tmp = tempfile::tempdir().unwrap();
    let broken = broken_project(tmp.path());
    let v = client.call_tool(6, "check_schema", json!({"path": broken.to_str().unwrap()}));
    assert_eq!(v["ok"], json!(false), "broken check: {v}");
    assert!(v["error_count"].as_u64().unwrap() >= 2);
    let codes: Vec<&str> = v["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["code"].as_str())
        .collect();
    assert!(codes.contains(&"E1101"), "codes: {codes:?}");
    assert!(codes.contains(&"E1201"), "codes: {codes:?}");
    for e in v["errors"].as_array().unwrap() {
        assert!(
            e["file"].as_str().is_some_and(|f| f.ends_with("Item.csv")),
            "file: {e}"
        );
        assert!(
            e["where"].as_str().is_some_and(|w| w.contains("Item.csv")),
            "where: {e}"
        );
        assert!(
            e["suggestion"].as_str().is_some_and(|s| !s.is_empty()),
            "suggestion: {e}"
        );
    }

    // 真调用 5：build_dry_run（好工程；预演不落盘）
    let v = client.call_tool(
        7,
        "build_dry_run",
        json!({"path": example_str, "profile": "client"}),
    );
    assert_eq!(v["ok"], json!(true), "dry run: {v}");
    assert_eq!(v["written"], json!(false));
    assert!(v["artifact_count"].as_u64().unwrap() > 0);
    assert!(v["schema_hash"].as_str().is_some_and(|s| !s.is_empty()));

    // 真调用 6：explain_config（配置语义 + 优先级链）
    let v = client.call_tool(8, "explain_config", json!({"path": example_str}));
    assert!(v["precedence"].as_array().is_some_and(|p| p.len() >= 5));
    assert!(v["profiles"]
        .as_object()
        .is_some_and(|p| p.contains_key("client")));
}

#[test]
fn unknown_env_returns_structured_error_with_suggestion() {
    let mut client = McpClient::spawn();
    client.request(
        1,
        "initialize",
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "cage-itest", "version": "0"}
        }),
    );
    writeln!(
        client.stdin,
        "{{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}}"
    )
    .unwrap();
    client.stdin.flush().unwrap();

    // 环境未声明：结构化错误 + 怎么修（在 env_overrides 里声明）
    let v = client.call_tool(
        2,
        "check_schema",
        json!({"path": example_project().to_str().unwrap(), "env": "qa"}),
    );
    assert_eq!(v["ok"], json!(false), "output: {v}");
    let msg = v["error"]["message"].as_str().expect("message");
    assert!(msg.contains("unknown environment 'qa'"), "msg: {msg}");
    let suggestion = v["error"]["suggestion"].as_str().expect("suggestion");
    assert!(
        suggestion.contains("env_overrides"),
        "suggestion: {suggestion}"
    );
}
