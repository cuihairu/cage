# CLI

Cage 的命令行入口（`cage-cli` crate，基于 [clap](https://crates.io/crates/clap)）：

```bash
cage check
cage build
cage gen
cage inspect
cage diff
cage verify
cage graph
cage web
```

MVP 落地前四个（`check` / `build` / `inspect` / `diff`），`gen` / `graph` 为
第二阶段（均已实装），`verify` 仍为第二阶段，`web` 为第三阶段
（Schema 编辑器本地服务）。

## check

```bash
cage check config/
```

只验证，不生成 Runtime Artifact。可以分级执行：

```bash
cage check --level schema       # 只做 Schema 层
cage check --level reference    # 只做到引用层
cage check --profile client     # 按 Profile 验证
```

## build

```bash
cage build config/ --profile client
cage build config/ --profile server
cage build config/ --incremental     # 哈希与上次 manifest 一致时跳过重新生成
```

验证并生成目标产物，同时输出 [Build Manifest](/build#build-manifest)。
`--incremental` 依据 manifest 里的 schema/source 哈希跳过未变化的重建
（校验仍全量执行；改了 cage.toml 的 targets 请全量重建，详见
[增量构建](/build#增量构建)）。

## gen

```bash
cage gen config/ --profile server
```

只生成代码类产物（[C# / Python / Lua / TypeScript / JavaScript / C++ / Go / Java](/target#code-targets-与-data-targets-分离)），
不跑数据校验：代码生成是 Schema 驱动的，类型与元数据全部来自 Schema，
不依赖配置行数据。Profile 里的数据类 Target（json/csv）会被跳过——需要
数据产物时用 `cage build`。产物同样写入 Build Manifest（与 build 同一口
径），后写者胜。

## inspect

```bash
cage inspect Item
```

查看 Schema 和配置结构。

## diff

```bash
cage diff build/a build/b
```

比较两个配置版本。

## verify

```bash
cage verify runtime/
```

验证已经生成的 Artifact（第二阶段）。

## graph

```bash
cage graph
```

输出[配置依赖图](/build#配置依赖图)（第二阶段）。

## web

```bash
cage web ./ --port 8765
```

启动 Schema 编辑器本地服务（第三阶段），只绑定 `127.0.0.1`、无需鉴权。
编辑器单页（W3 实装）与 HTTP API 共用一份交换文档，API 契约见
[Web UI](/web)：

```text
GET  /api/schema      → 合并 Schema 的编辑器文档（每请求重载，保存后立即可见）
POST /api/validate    → 编辑态校验（E1701 / E1004 诊断）
POST /api/schema      → 保存回环 canonical YAML（单文件 schema_path 才可写）
```

Ctrl+C 停止服务。`--port` 默认 8765。

## 快速开始

```bash
cage check config/                 # 只验证，不生成
cage build config/ --profile client  # 验证并生成目标产物
cage gen config/ --profile server    # 只生成代码绑定（cs/python/lua/ts/js/cpp/go/java）
cage diff build/a build/b          # 比较两个配置版本
cage inspect Item                  # 查看 Schema 与配置结构
```
