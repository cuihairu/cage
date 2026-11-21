// 编辑器状态单例（split out of app.js, LAY-027）。

// ── 状态 ────────────────────────────────────────────────────────────

const state = {
  meta: null,        // GET /api/schema 负荷
  doc: null,         // 编辑器文档（canonical serde 形状），原地修改
  sel: null,         // {kind: "table" | "enum", name}
  field: null,       // 当前表内选中的字段名
  diags: [],         // 最近一次校验结果
  dirty: false,
  scope: null,       // profile 作用域（U2b）：{profile, hiddenTables, hiddenFields}；null = 全部
  mode: "basic",     // 编辑模式（P1-1）："basic" | "rules" | "expert"，三层共享一份 state.doc
};

const enumNames = () => (state.doc ? Object.keys(state.doc.enums || {}) : []);

// 表名列表（文档顺序）。exclude 传入当前正在编辑的表名，用于引用下拉
// 排除自身，避免自引用默认值（REF-FIX-001）。
const tableNames = (exclude) => {
  if (!state.doc) return [];
  return Object.keys(state.doc.tables || {}).filter((n) => n !== exclude);
};


export { state, enumNames, tableNames };
