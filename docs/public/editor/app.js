// 编辑器入口：校验/保存/诊断/作用域/启动（split out of app.js, LAY-027）。

import { state } from "./state.js";
import { el, nextName, option, setPathRoot, toast } from "./dom.js";
import { api, markDirty, resetAutoValidate } from "./api.js";
import { refreshEntity, renderTree } from "./forms.js";
import { MODE_STORAGE_KEY, applyExpertText, isMode } from "./modes.js";

// ── 校验与保存 ──────────────────────────────────────────────────────

async function validate(silent) {
  resetAutoValidate();
  const docText = JSON.stringify(state.doc);
  const { status, data } = await api("/api/validate", docText);
  void status;
  state.diags = (data && data.diagnostics) || [];
  renderDiags(false);
  if (silent) return; // 自动重校验只更新诊断面板，不弹 toast
  const errs = state.diags.filter((d) => d.severity === "error").length;
  const warns = state.diags.filter((d) => d.severity === "warning").length;
  if (data && data.ok) toast("校验通过（L1 无错误）", "ok");
  else toast(`校验未通过：${errs} 错误 · ${warns} 警告`, "err");
}

/// Build the save payload (envelope for multi-file, raw document for
/// single-file).
function savePayload() {
  return state.meta && state.meta.sources
    ? { schema: state.doc, sources: state.meta.sources }
    : state.doc;
}

/// POST /api/schema — the real write. Adopts the returned index and clears
/// the dirty flag on success.
async function performSave(payload) {
  const docText = JSON.stringify(payload);
  const { status, data } = await api("/api/schema", docText);
  if (data && data.ok) {
    if (data.sources && state.meta) state.meta.sources = data.sources;
    state.dirty = false;
    document.getElementById("dirty-flag").hidden = true;
    toast(`已保存 → ${data.save_target}（${data.bytes} 字节）`, "ok", 6000);
    renderDiags(false);
    if (data.note) toast(`已保存，注意：${data.note}`, "ok", 9000);
  } else if (data && data.diagnostics) {
    state.diags = data.diagnostics;
    renderDiags(false);
    toast("保存被拒：编辑文档未通过 E1701（见诊断面板）", "err");
  } else {
    const msg = (data && (data.error || `HTTP ${status}`)) || "保存失败";
    toast(`保存失败：${msg}`, "err", 7000);
  }
}

/// One diff file as a colored <pre> block (per-line span by prefix).
function diffHtml(diff) {
  const pre = el("pre", "diff-text");
  diff.split("\n").forEach((ln) => {
    const cls =
      ln.startsWith("+") && !ln.startsWith("+++") ? "diff-add"
        : ln.startsWith("-") && !ln.startsWith("---") ? "diff-del"
          : ln.startsWith("@@") ? "diff-hunk"
            : ln.startsWith("---") || ln.startsWith("+++") ? "diff-meta"
              : "";
    pre.append(el("span", cls, ln), document.createTextNode("\n"));
  });
  return pre;
}

/// Show the save-diff modal; confirm runs the real save, cancel/close just
/// dismisses. Listeners are reassigned per open, so no stacking.
function showDiffModal(payload, preview) {
  const modal = document.getElementById("diff-modal");
  document.getElementById("diff-meta").textContent =
    `将改写 ${preview.files.length} 个文件 · ${preview.bytes} 字节 · 未改动 ${preview.unchanged} 个`;
  const filesBox = document.getElementById("diff-files");
  filesBox.replaceChildren();
  (preview.files || []).forEach((f) => {
    const block = el("div", "diff-file");
    const head = el("div", "diff-file-head");
    head.append(
      el("span", "diff-file-name", f.file),
      el("span", "diff-file-action", f.action === "create" ? "新建" : "改写")
    );
    block.append(head, diffHtml(f.diff));
    filesBox.append(block);
  });
  modal.hidden = false;
  const close = () => { modal.hidden = true; };
  document.getElementById("diff-confirm").onclick = () => { close(); performSave(payload); };
  document.getElementById("diff-cancel").onclick = close;
  document.getElementById("diff-close").onclick = close;
}

/// Ctrl/Cmd+S — first preview the write (the exact diff the save would
/// make, computed server-side without writing), then on confirm perform it.
/// An empty preview saves directly (nothing to review).
async function save() {
  const payload = savePayload();
  const docText = JSON.stringify(payload);
  const { status, data } = await api("/api/schema/preview", docText);
  if (data && data.ok) {
    if (data.files && data.files.length) showDiffModal(payload, data);
    else performSave(payload);
  } else if (data && data.diagnostics) {
    state.diags = data.diagnostics;
    renderDiags(false);
    toast("保存被拒：编辑文档未通过 E1701（见诊断面板）", "err");
  } else {
    const msg = (data && (data.error || `HTTP ${status}`)) || "预览失败";
    toast(`保存预览失败：${msg}`, "err", 7000);
  }
}

/// Ctrl/Cmd+Shift+Enter — the full precheck (P0-3): the server loads this
/// draft against the project's real data through the very pipeline
/// `cage check` runs, and answers with one merged diagnostic list. A
/// scope badge per finding (结构 = pure structural, 数据 = data-side) is the
/// merge口径: the fast「校验」above stays editor-state only, because data
/// loading cannot ride the 700ms auto-revalidate.
async function precheck() {
  resetAutoValidate();
  const docText = JSON.stringify(state.doc);
  const { status, data } = await api("/api/validate/data", docText);
  void status;
  state.diags = (data && data.diagnostics) || [];
  renderDiags(false);
  if (!data) return;
  const lane = `${data.level || "?"} · ${data.profile || "?"}`;
  if (data.ok) toast(`预检通过：草稿 Schema 与真实数据一致（${lane}）`, "ok");
  else if (data.error) toast(`预检失败：${data.error}`, "err", 9000);
  else toast(`预检未通过：${data.errors || 0} 错误 · ${data.warnings || 0} 警告（${lane}）`, "err", 9000);
}

/// The safe rename (P0-4): the server runs `cage_core::refactor::apply` on
/// this draft — the one place that knows every site a name appears in
/// (primary key, unique constraints, `order_by`, environment overrides,
/// cross-table references and their predicates, `Enum(name)` types, rule
/// operands) — and answers with the renamed document plus the change report.
/// The author sees that report before the draft adopts it; declining
/// discards it. The server never writes to disk here, so an unsaved draft is
/// never lost to a rename. `target` is `{kind:"table",name}` /
/// `{kind:"field",table,field}` / `{kind:"enum",name}`.
async function apiRename(target, newName) {
  const name = (newName || "").trim();
  if (!name) return false;
  const docText = JSON.stringify({ schema: state.doc, target, new_name: name });
  const { status, data } = await api("/api/schema/rename", docText);
  void status;
  if (!data) return false;
  if (data.diagnostics) { // E1701: the draft does not even parse into a Schema
    state.diags = data.diagnostics;
    renderDiags(false);
    toast("改名被拒：编辑文档未通过 E1701（见诊断面板）", "err");
    return false;
  }
  if (!data.ok) { // collision / ghost target / non-identifier new name
    toast(`改名失败：${data.error}`, "err", 7000);
    return false;
  }
  const lines = data.changes.map((c) => `${c.kind}: ${c.path}`).join("\n");
  const notes = data.notes && data.notes.length ? `\n\n未处理（保持不变）：\n${data.notes.join("\n")}` : "";
  const ok = window.confirm(`「改名」将同步更新 ${data.changes.length} 处：\n\n${lines}${notes}\n\n应用吗？`);
  if (!ok) return false;
  state.doc = data.schema;
  if (target.kind === "table") state.sel = { kind: "table", name: data.new_name };
  else if (target.kind === "field") {
    state.sel = { kind: "table", name: target.table };
    state.field = data.new_name;
  } else state.sel = { kind: "enum", name: data.new_name };
  refreshEntity();
  markDirty();
  toast(`已改名：${data.changes.length} 处同步更新`, "ok", 6000);
  return true;
}

// ── 诊断面板 ────────────────────────────────────────────────────────

function flash(node) {
  node.classList.remove("flash");
  void node.offsetWidth;
  node.classList.add("flash");
}

function renderDiags(initial) {
  const list = document.getElementById("diag-list");
  const count = document.getElementById("diag-count");
  list.replaceChildren();
  if (!state.diags || state.diags.length === 0) {
    count.textContent = initial ? "" : "通过";
    const li = el("li", "diag", initial ? "尚未校验——点「校验」跑编辑态检查，「完整预检」跑数据口径（E1701 / E1004）" : "无诊断");
    li.style.cursor = "default";
    list.append(li);
    return;
  }
  const errs = state.diags.filter((d) => d.severity === "error").length;
  const warns = state.diags.filter((d) => d.severity === "warning").length;
  count.textContent = `${errs} 错误 · ${warns} 警告`;
  state.diags.forEach((d) => {
    const li = el("li", `diag severity-${d.severity || "info"}`);
    const line1 = el("div", "diag-line1");
    line1.append(el("span", "diag-code", d.code || "?"));
    if (d.scope) line1.append(el("span", `diag-scope scope-${d.scope}`, d.scope === "data" ? "数据" : "结构"));
    line1.append(el("div", "diag-msg", d.message || ""));
    li.append(line1);
    if (d.field && d.field !== ".") li.append(el("div", "diag-field", d.field));
    if (d.hint) li.append(el("div", "diag-hint", d.hint));
    li.addEventListener("click", () => focusDiag(d));
    list.append(li);
  });
}

function focusDiag(d) {
  const field = d.field || "";
  const seg = field.split(".");
  // A data-side finding (the precheck's 数据 scope) names its table outright
  // and its column; an editor-state one carries a JSON path instead.
  const table = d.table || (seg[0] === "tables" ? seg[1] : null);
  const column = seg[3] === "fields" && seg[4] ? seg[4] : (d.table ? (d.column || field) : null);
  const enumName = !table && seg[0] === "enums" ? seg[1] : null;
  if (table && state.doc.tables[table]) {
    state.sel = { kind: "table", name: table };
    if (column && state.doc.tables[table].fields[column]) state.field = column;
    refreshEntity();
  } else if (enumName && state.doc.enums[enumName]) {
    state.sel = { kind: "enum", name: enumName };
    refreshEntity();
  } else if (!table && !enumName && (!field || field === ".")) {
    flash(document.getElementById("diag-list"));
    return;
  } else {
    refreshEntity();
  }
  const exact = [...document.querySelectorAll("[data-path]")]
    .find((n) => n.getAttribute("data-path") === field);
  if (exact) {
    exact.scrollIntoView({ behavior: "smooth", block: "center" });
    flash(exact);
  }
}

// ── 作用域（profile 投影视图，U2b）──────────────────────────────────
//
// 语义全部在服务端：GET /api/schema?profile=NAME 用 filter_by_profile
// （构建管线同一函数）出投影版。前端只做集合差（全量 − 投影 = 隐藏集）
// 并据此灰显——可见性规则不进 JS。隐藏项不消失，而是带「本 profile
// 不可见」徽标：藏起来的东西要看得见被藏了。

function applyScope(profile) {
  if (!profile) {
    state.scope = null;
    updateScopeNote();
    refreshEntity();
    return;
  }
  api(`/api/schema?profile=${encodeURIComponent(profile)}`).then(({ data }) => {
    if (!data || !data.ok || !data.projection) {
      toast(`作用域装载失败：${(data && data.error) || "服务错误"}`, "err");
      return;
    }
    // 对比服务器这份全量文档（state.doc 可能含未保存改动，不参与计算）。
    const full = data.schema.tables || {};
    const proj = data.projection.schema.tables || {};
    const hiddenTables = new Set(Object.keys(full).filter((t) => !proj[t]));
    const hiddenFields = new Map();
    for (const [t, td] of Object.entries(full)) {
      const pt = proj[t];
      if (!pt) continue;
      const hid = Object.keys(td.fields || {}).filter((f) => !pt.fields[f]);
      if (hid.length) hiddenFields.set(t, new Set(hid));
    }
    state.scope = { profile, hiddenTables, hiddenFields };
    updateScopeNote();
    refreshEntity();
  });
}

function updateScopeNote() {
  const note = document.getElementById("scope-note");
  if (!state.scope) {
    note.hidden = true;
    return;
  }
  note.hidden = false;
  const caveat = state.dirty ? "（按已保存 schema 投影）" : "";
  note.textContent = `作用域 ${state.scope.profile}${caveat}：灰显项对该 profile 不可见`;
}

// ── 编辑模式（P1-1：基础/规则/专家共享一份语义模型）───────────────────
//
// 三层编辑模式（评审 P1-1）：基础=字段表单（forms.js）；规则=表达式规则
// 视图（forms.js renderRulesMode）；专家=原始文档（本模块）。三者读写
// 同一份 state.doc、同一个 field.rules 数组——禁三套规则格式的契约在
// modes.js（node --test 单测锁死）。

function updateModeSwitch() {
  for (const b of document.querySelectorAll("#mode-switch .mode-btn")) {
    const on = b.dataset.mode === state.mode;
    b.classList.toggle("active", on);
    b.setAttribute("aria-selected", on ? "true" : "false");
  }
}

function setMode(m) {
  if (!isMode(m) || m === state.mode) return;
  state.mode = m;
  try { localStorage.setItem(MODE_STORAGE_KEY, m); } catch { /* 隐私模式下静默放弃持久化 */ }
  updateModeSwitch();
  // 专家模式的整文档改写可能让现有选中失效：换模式前清掉幽灵选中。
  if (state.sel && state.doc) {
    const known = state.sel.kind === "enum"
      ? state.doc.enums && state.doc.enums[state.sel.name]
      : state.doc.tables && state.doc.tables[state.sel.name];
    if (!known) {
      state.sel = null;
      state.field = null;
    } else if (state.field) {
      const f = state.doc.tables[state.sel.name].fields;
      if (!(f && f[state.field])) state.field = null;
    }
  }
  refreshEntity();
}

/// 专家模式：直接编辑文档 JSON。文档就是 Schema 语义模型的 serde 形状
/// ——专家层编辑这一份，不是并行表示；保存仍由服务端写 canonical
/// YAML。「应用」走 modes.applyExpertText：解析失败原子拒绝，现有文档
/// 一字节不动；形状问题交给「校验」（E1701）报告。
function renderExpertMode() {
  const box = document.getElementById("entity");
  box.replaceChildren();
  box.append(el("h2", null, "专家模式 · 文档（JSON ≡ Schema 语义模型）"));
  const note = el("div", "type-kind-hint");
  note.textContent =
    "直接编辑 Schema 的 serde 形状（编辑器不解析、不渲染 YAML；保存仍写 canonical YAML）。"
    + "「应用」仅当文本是合法 JSON 对象时替换文档，失败时现有文档保留；"
    + "结构问题由「校验」（E1701）与「完整预检」报告。";
  box.append(note);
  const ta = el("textarea");
  ta.id = "expert-doc";
  ta.spellcheck = false;
  ta.value = JSON.stringify(state.doc, null, 2);
  const actions = el("div", "field-actions");
  const applyBtn = el("button", "primary", "应用");
  applyBtn.addEventListener("click", () => {
    const r = applyExpertText(ta.value);
    if (r.error) {
      ta.classList.add("json-bad");
      toast(r.error, "err", 7000);
      return;
    }
    ta.classList.remove("json-bad");
    state.doc = r.doc;
    markDirty();
    refreshEntity(); // 重渲染（含左栏树——文档可能换了表）
    validate();
  });
  const reloadBtn = el("button", "", "重新载入");
  reloadBtn.addEventListener("click", () => {
    ta.value = JSON.stringify(state.doc, null, 2);
    ta.classList.remove("json-bad");
  });
  actions.append(applyBtn, reloadBtn);
  box.append(ta, actions);
}

// ── 导入数据文件 → Schema 草稿（P1-6）──────────────────────────────

/// The draft the preview holds (editor JSON straight from the endpoint);
/// null until a draft was generated in this modal session.
let importedDraft = null;

/// Render the inferred draft as a per-table field list: name (primary key
/// marked), type, required, enum values.
function renderImportPreview(schema) {
  const box = el("div");
  for (const [tName, table] of Object.entries((schema && schema.tables) || {})) {
    const tbl = el("table");
    const head = el("tr");
    for (const h of ["字段", "类型", "必填", "枚举/说明"]) head.append(el("th", null, h));
    tbl.append(head);
    for (const f of Object.values((table && table.fields) || {})) {
      const isPk = ((table && table.primary_key) || []).includes(f.name);
      const tr = el("tr");
      tr.append(el("td", isPk ? "pk" : null, isPk ? `${f.name} 🔑` : f.name));
      tr.append(el("td", null, (f.type && f.type.kind) || "?"));
      tr.append(el("td", null, f.required ? "必填" : "可空"));
      tr.append(
        el("td", "enum",
          f.enum_values ? `枚举: ${f.enum_values.join(", ")}` : (f.description || ""))
      );
      tbl.append(tr);
    }
    const block = el("div", "diff-file");
    const h = el("div", "diff-file-head");
    h.append(
      el("span", "diff-file-name", tName),
      el("span", "diff-file-action",
        `主键 ${((table && table.primary_key) || []).join(", ") || "—"} · `
        + `${Object.keys((table && table.fields) || {}).length} 字段`)
    );
    block.append(h, tbl);
    box.append(block);
  }
  return box;
}

function openImportModal() {
  const modal = document.getElementById("import-modal");
  importedDraft = null;
  document.getElementById("import-path").value = "";
  const preview = document.getElementById("import-preview");
  preview.replaceChildren();
  preview.hidden = true;
  const warn = document.getElementById("import-warnings");
  warn.replaceChildren();
  warn.hidden = true;
  document.getElementById("import-apply").disabled = true;
  // The project's known data files seed the path datalist (the same table
  // inventory the data panel shows), so the planner picks rather than types.
  api("/api/tables").then(({ data }) => {
    const dl = document.getElementById("import-files");
    dl.replaceChildren();
    const seen = new Set();
    for (const t of (data && data.tables) || []) {
      if (t.source_file && !seen.has(t.source_file)) {
        seen.add(t.source_file);
        dl.append(option(t.source_file));
      }
    }
  });
  modal.hidden = false;
}

async function runImport() {
  const path = document.getElementById("import-path").value.trim();
  if (!path) return toast("请先填写数据文件路径", "err");
  const { status, data } = await api("/api/import/schema", JSON.stringify({ path }));
  const preview = document.getElementById("import-preview");
  const warn = document.getElementById("import-warnings");
  if (!data || !data.ok) {
    importedDraft = null;
    preview.replaceChildren();
    preview.hidden = true;
    warn.replaceChildren(el("div", null, (data && data.error) || `HTTP ${status}`));
    warn.hidden = false;
    document.getElementById("import-apply").disabled = true;
    return;
  }
  importedDraft = data.schema;
  preview.replaceChildren(renderImportPreview(data.schema));
  preview.hidden = false;
  warn.replaceChildren(...(data.warnings || []).map((w) => el("div", null, `· ${w}`)));
  warn.hidden = !(data.warnings || []).length;
  document.getElementById("import-apply").disabled = false;
}

function applyImport() {
  if (!importedDraft) return;
  state.doc = importedDraft;
  importedDraft = null;
  document.getElementById("import-modal").hidden = true;
  markDirty();
  refreshEntity(); // 文档可能换了表：左栏树一起重渲染
  validate();
  toast("已应用导入草稿：校验通过后「保存」写回 canonical YAML", "ok", 6000);
}

function closeImportModal() {
  document.getElementById("import-modal").hidden = true;
}

// ── 启动 ────────────────────────────────────────────────────────────

async function init() {
  const { data } = await api("/api/schema");
  if (!data || !data.ok) {
    document.getElementById("fatal-error").hidden = false;
    document.getElementById("fatal-error").textContent =
      `无法装载 schema：${(data && (data.error || "服务错误")) || "无法连接"}`;
    renderTree();
    return;
  }
  state.meta = data;
  state.doc = data.schema;

  document.getElementById("meta-project").textContent = data.project || "?";
  document.getElementById("meta-path").textContent = data.schema_path || "未配置 schema_path";
  document.getElementById("meta-profiles").textContent =
    `profiles: ${(data.profile_names || []).join(", ") || "（无）"}`;
  if (data.warnings_as_errors) document.getElementById("meta-wae").hidden = false;

  // 作用域旋钮（U2b）：切 profile 出投影视图
  const scopeSel = document.getElementById("scope-sel");
  for (const p of data.profile_names || []) scopeSel.append(option(p));
  scopeSel.addEventListener("change", () => applyScope(scopeSel.value));

  // 编辑模式（P1-1）：基础/规则/专家三层，共享一份 Schema 语义模型。
  const savedMode = (() => {
    try { return localStorage.getItem(MODE_STORAGE_KEY); } catch { return null; }
  })();
  if (isMode(savedMode)) state.mode = savedMode;
  updateModeSwitch();
  for (const b of document.querySelectorAll("#mode-switch .mode-btn")) {
    b.addEventListener("click", () => setMode(b.dataset.mode));
  }

  // 枚举名 datalist（枚举类型引用输入用）
  const dl = el("datalist");
  dl.id = "enum-names";
  document.body.append(dl);

  document.getElementById("add-table").addEventListener("click", () => {
    const name = nextName(state.doc.tables, "new_table");
    setPathRoot(["tables"], { ...state.doc.tables, [name]: { name, description: null, primary_key: [], fields: {} } });
    state.sel = { kind: "table", name };
    refreshEntity();
    validate();
  });
  document.getElementById("add-enum").addEventListener("click", () => {
    const name = nextName(state.doc.enums, "new_enum");
    setPathRoot(["enums"], { ...state.doc.enums, [name]: { name, description: null, values: [] } });
    state.sel = { kind: "enum", name };
    refreshEntity();
  });
  document.getElementById("validate-btn").addEventListener("click", validate);
  document.getElementById("precheck-btn").addEventListener("click", precheck);
  document.getElementById("save-btn").addEventListener("click", save);
  document.getElementById("import-btn").addEventListener("click", openImportModal);
  document.getElementById("import-run").addEventListener("click", runImport);
  document.getElementById("import-apply").addEventListener("click", applyImport);
  document.getElementById("import-cancel").addEventListener("click", closeImportModal);
  document.getElementById("import-close").addEventListener("click", closeImportModal);
  document.getElementById("import-backdrop").addEventListener("click", closeImportModal);

  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "s") {
      e.preventDefault();
      save();
    } else if ((e.metaKey || e.ctrlKey) && e.key === "Enter" && e.shiftKey) {
      e.preventDefault();
      precheck();
    } else if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      validate();
    }
  });
  window.addEventListener("beforeunload", (e) => {
    if (state.dirty) { e.preventDefault(); e.returnValue = ""; }
  });

  refreshEntity();
  renderDiags(true);
  validate();
}

init();

export { validate, precheck, save, apiRename, performSave, savePayload, showDiffModal, flash, renderDiags, focusDiag, applyScope, updateScopeNote, renderExpertMode, setMode, updateModeSwitch, init };
