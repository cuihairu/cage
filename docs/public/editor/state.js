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
};

const enumNames = () => (state.doc ? Object.keys(state.doc.enums || {}) : []);


export { state, enumNames };
