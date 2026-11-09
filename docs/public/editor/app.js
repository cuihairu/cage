// 编辑器入口：校验/保存/诊断/作用域/启动（split out of app.js, LAY-027）。

import { state } from "./state.js";
import { el, nextName, option, setPathRoot, toast } from "./dom.js";
import { api, resetAutoValidate } from "./api.js";
import { refreshEntity, renderTree } from "./forms.js";

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

async function save() {
  // 多文件 schema（schema_path 指目录）走保存信封：把 GET 返回的
  // sources 索引（每份源文件的 sha256 revision + 表/枚举归属）一并交回，
  // 服务端做冲突检查后只改写涉及变更的源文件；单文件 schema 沿用
  // 原样提交整份编辑文档。保存成功后采纳服务端回传的新索引。
  const payload = state.meta && state.meta.sources
    ? { schema: state.doc, sources: state.meta.sources }
    : state.doc;
  const docText = JSON.stringify(payload);
  const { status, data } = await api("/api/schema", docText);
  if (data && data.ok) {
    if (data.sources && state.meta) state.meta.sources = data.sources;
    state.dirty = false;
    document.getElementById("dirty-flag").hidden = true;
    toast(`已保存 → ${data.save_target}（${data.bytes} 字节）`, "ok", 6000);
    // 校验通过才允许静默；此处不自动校验（作者主导）
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
    const li = el("li", "diag", initial ? "尚未校验——点「校验」跑编辑态检查（E1701 / E1004）" : "无诊断");
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
    line1.append(el("span", "diag-code", d.code || "?"), el("div", "diag-msg", d.message || ""));
    li.append(line1);
    if (d.field && d.field !== ".") li.append(el("div", "diag-field", d.field));
    if (d.hint) li.append(el("div", "diag-hint", d.hint));
    li.addEventListener("click", () => focusDiag(d));
    list.append(li);
  });
}

function focusDiag(d) {
  const field = d.field || "";
  if (!field || field === ".") { flash(document.getElementById("diag-list")); return; }
  const seg = field.split(".");
  if (seg[0] === "tables" && seg[1] && state.doc.tables[seg[1]]) {
    state.sel = { kind: "table", name: seg[1] };
    if (seg[3] === "fields" && seg[4] && state.doc.tables[seg[1]].fields[seg[4]]) state.field = seg[4];
    refreshEntity();
  } else if (seg[0] === "enums" && seg[1] && state.doc.enums[seg[1]]) {
    state.sel = { kind: "enum", name: seg[1] };
    refreshEntity();
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
  document.getElementById("save-btn").addEventListener("click", save);

  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "s") {
      e.preventDefault();
      save();
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

export { validate, save, flash, renderDiags, focusDiag, applyScope, updateScopeNote, init };
