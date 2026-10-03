# Cage TODO

> 依据 docs/design.md + docs/需求整理.md。每项可独立验收，完成即勾。

## 第一期 MVP

### T1 工程骨架
- [x] T1.1 Cargo workspace 起（cage-core/source-*/target-*/cli 多 crate + CI）
- [x] T1.2 Canonical Model（value.rs：Null/Bool/Int/UInt/Float/String/Bytes/Array/Object + Source Location）
- [x] T1.3 Diagnostics 框架（错误码表 + 全字段诊断 + 多级 Severity + 文本渲染器）

### T2 Source
- [x] T2.1 JSON Source Adapter（serde 读入 → Canonical Model，保留路径定位）
- [x] T2.2 YAML Source Adapter（语法错误 E0001 精确到行）
- [x] T2.3 CSV Source Adapter（表头/行号定位）
- [x] T2.4 Excel Source Adapter（calamine：sheet/合并单元格/缓存值；错误定位到单元格 G27 式）

### T3 Schema 与校验
- [x] T3.1 Schema DSL（YAML 定义：type/required/default/enum/min/max/unique/primary_key/reference）
- [x] T3.2 L1 Schema + L2 Type + L3 Value 校验（E1001/E1002/E1101/E1201 族）
- [x] T3.3 L4 Table（主键唯一 E1301、组合唯一）
- [x] T3.4 L5 Reference（E1401 存在性；E1410 谓词：引用对象字段约束）
- [x] T3.5 L6 Semantic 表达式规则（assert: min_level <= max_level，行级定位）
- [x] T3.6 `--level` 分级执行

### T4 Normalize 与 Target
- [x] T4.1 Normalize（数值/布尔归一，确定性 Canonical Representation）
- [x] T4.2 JSON Target（稳定键序、确定性字节输出）
- [x] T4.3 CSV Target
- [x] T4.4 Profile 机制（client/server 配置 + 字段级 targets 可见性过滤）

### T5 Build 与 CLI
- [x] T5.1 确定性构建 + Build Manifest（schema_hash/source_hash/content_hash/artifacts）
- [x] T5.2 CLI：cage check / build / inspect / diff（clap）
- [x] T5.3 golden 测试：同输入两次构建字节级一致

## 第二阶段（预排）
- [ ] 代码生成器：C#/Python/Lua
- [ ] Plugin SDK + Game Rule Validator 实装（trait + 动态库/脚本沙箱方案定稿）
- [x] Dependency Graph（cage graph：引用图/构建顺序/循环检测）
- [ ] 增量构建（变更影响分析）
- [x] CI 集成（warnings_as_errors、GitHub Actions 模板）

## 第三阶段（预排）
- [ ] Web UI / Schema Editor
- [ ] Registry（远程配置仓库 + 版本）
- [ ] Remote Source（Google Sheets/MySQL/PostgreSQL/HTTP API）
- [ ] Artifact 分发与迁移
