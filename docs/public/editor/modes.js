// modes.js — P1-1 三层编辑模式的纯逻辑（node --test 可测，零依赖、无 DOM）。
//
// 三个模式共享一份 Schema 语义模型（state.doc，canonical serde 形状）：
// 基础模式 = 字段表单；规则模式 = 表达式规则视图；专家模式 = 原始文档。
// 三者读写同一个 `field.rules`（ExpressionRule 数组）——不存在第二套
// 规则格式（禁三套规则格式）。本模块只放可单元测试的契约，DOM 渲染在
// app.js / forms.js。

export const MODES = ["basic", "rules", "expert"];

/// localStorage 键（持久化作者上次选择的模式）。
export const MODE_STORAGE_KEY = "cage-editor-mode";

export function isMode(m) {
  return MODES.includes(m);
}

/// 专家模式：文档文本 → Schema 语义模型。原子——任何失败返回 {error}，
/// 绝不产出部分文档；调用方失败时不得改写 state.doc（现有文档原样保留）。
export function applyExpertText(text) {
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch (e) {
    return { error: `JSON 无法解析：${e.message}` };
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return { error: "文档必须是 JSON 对象（Schema 语义模型的 serde 形状）" };
  }
  return { doc: parsed };
}

/// 单一规则表示的读取口：字段的 rules 数组。基础/规则/专家三模式都经
/// 此处读写同一数组，保证没有第二套格式。
export function ruleFormat(doc, table, field) {
  const f = doc && doc.tables && doc.tables[table] && doc.tables[table].fields
    && doc.tables[table].fields[field];
  return f ? f.rules : undefined;
}

/// 规则模式新增：在唯一 `field.rules` 数组上追加一条 ExpressionRule。
/// 原地改传入文档（与 setPathRoot 相同的文档原地语义），返回 {ok, doc}；
/// 未知字段返回 {ok:false, error}。
export function addRule(doc, table, field) {
  const f = doc && doc.tables && doc.tables[table] && doc.tables[table].fields
    && doc.tables[table].fields[field];
  if (!f) return { ok: false, error: `未知字段 ${table}.${field}` };
  const items = Array.isArray(f.rules) ? f.rules : [];
  f.rules = [...items, { name: `rule_${items.length + 1}`, assert: "" }];
  return { ok: true, doc };
}
