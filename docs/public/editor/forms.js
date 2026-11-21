// 表单渲染：表/字段/枚举/树 与模板库（split out of app.js, LAY-027）。

import { apiRename, renderExpertMode } from "./app.js";
import { api, markDirty } from "./api.js";
import { FIELD_KEYS, NUMERIC_KINDS, STRING_KIND } from "./constants.js";
import { builderRowsToAst, mergeHumanLayer, parseLiteralInput } from "./modes.js";
import { RULE_TEMPLATES, boundaryGuidance, planTemplate, templateById } from "./rule_templates.js";
import { enumNames, state, tableNames } from "./state.js";
import { boolInput, delPathRoot, el, getPath, intInput, jsonInput, listInput, nextName, numberInput, option, setPathRoot, targetsChips, textInput, toast } from "./dom.js";
import { TypeEditor, typeLabel } from "./types.js";

// ── 表单渲染 ────────────────────────────────────────────────────────

function entitySection(title) {
  const sec = el("section", "section");
  sec.append(el("h3", null, title));
  return sec;
}

function entityRow(labelText, ctl, dataPath) {
  const row = el("div", "row");
  row.append(el("label", null, labelText));
  if (dataPath) ctl.setAttribute("data-path", dataPath);
  row.append(ctl);
  return row;
}

/// 改名称统一走服务端的安全重命名（P0-4，`POST /api/schema/rename`）：
/// 表/字段/枚举的键名与自身 `name` 成员、主键、唯一约束、排序、env_overrides、
/// 跨表引用（及其谓词）、Enum 类型、规则表达式逐处同步——这些站点由
/// `cage_core::refactor` 一次性给出，作者先看到清单再采纳。改名只改文档，
/// 不碰磁盘；被拒时把输入框还原回原名。
async function renameEntity(target, oldName, input) {
  const newName = input.value.trim();
  if (!newName) { input.value = oldName; return; }
  const done = await apiRename(target, newName);
  if (!done) input.value = oldName;
}

// 表编辑器
function renderTableForm() {
  const box = document.getElementById("entity");
  box.replaceChildren();
  const table = state.doc.tables[state.sel.name];
  if (!table) return;

  const tPath = ["tables", state.sel.name];
  const title = el("h2", null, `表 · ${state.sel.name}`);
  const titleChips = targetsChips(table.targets);
  if (titleChips) title.append(titleChips);
  box.append(title);

  // —— 表定义 ——
  const def = entitySection("表定义");
  const nameInput = textInput(tPath.concat("name"));
  nameInput.setAttribute("data-path", tPath.concat("name").join("."));
  nameInput.addEventListener("change", () =>
    renameEntity({ kind: "table", name: state.sel.name }, state.sel.name, nameInput));
  def.append(entityRow("名称", nameInput));
  def.append(entityRow("描述", textInput(tPath.concat("description"), { toNull: true })));
  def.append(entityRow("主键", listInput(tPath.concat("primary_key")), tPath.concat("primary_key").join(".")));
  def.append(entityRow("排序", listInput(tPath.concat("order_by"))));
  def.append(entityRow("Targets（profile 可见性）", listInput(tPath.concat("targets"))));
  box.append(def);

  // —— 唯一约束 ——
  const uniq = entitySection("唯一约束");
  const uniqList = el("div", "nested-list");
  uniqList.setAttribute("data-path", tPath.concat("unique_constraints").join("."));
  const uniqArr = table.unique_constraints || [];
  const renderUniq = () => {
    uniqList.replaceChildren();
    uniqArr.forEach((_, i) => {
      const row = el("div", "nested-row");
      const grip = el("div", "nested-grip");
      grip.append(el("span", null, `#${i}`));
      const delBtn = el("button", "icon danger small", "删除");
      delBtn.addEventListener("click", () => {
        delPathRoot(tPath.concat("unique_constraints", String(i)));
        refreshEntity();
      });
      grip.append(delBtn);
      row.append(grip);
      const grid = el("div", "nested-grid cols-2");
      grid.append(el("label", null, "约束名"),
        textInput(tPath.concat("unique_constraints", String(i), "name")));
      grid.append(el("label", null, "字段（逗号分隔）"),
        listInput(tPath.concat("unique_constraints", String(i), "fields")));
      row.append(grid);
      uniqList.append(row);
    });
  };
  renderUniq();
  const addUniq = el("button");
  addUniq.textContent = "＋ 唯一约束";
  addUniq.addEventListener("click", () => {
    const items = table.unique_constraints || [];
    setPathRoot(tPath.concat("unique_constraints"), [...items, { name: "uniq", fields: [] }]);
    refreshEntity();
  });
  uniq.append(uniqList, addUniq);
  box.append(uniq);

  // —— 字段 ——
  const fsec = entitySection("字段");
  const ftable = el("table", "fields-table");
  const thead = el("thead");
  const hr = el("tr");
  ["字段", "类型", "要求", ""].forEach((h) => hr.append(el("th", null, h)));
  thead.append(hr);
  ftable.append(thead);
  const tbody = el("tbody");
  const fields = table.fields || {};
  for (const fname of Object.keys(fields)) {
    const f = fields[fname];
    const tr = el("tr", "field-row");
    tr.classList.toggle("active", state.field === fname);
    tr.dataset.field = fname;
    const tdName = el("td", "name-cell", fname);
    const fHiddenSet = state.scope ? state.scope.hiddenFields.get(state.sel.name) : null;
    if (fHiddenSet && fHiddenSet.has(fname)) {
      tr.classList.add("scoped-hidden");
      tdName.append(el("span", "scope-badge", "本 profile 不可见"));
    }
    const fChips = targetsChips(f.targets);
    if (fChips) tdName.append(fChips);
    const tdType = el("td", "type-cell", typeLabel(f.type));
    const tdReq = el("td", "req-cell", f.required ? "必填" : "");
    const tdDel = el("td");
    const delBtn = el("button", "icon danger", "×");
    delBtn.title = "删除字段";
    delBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      delPathRoot(tPath.concat("fields", fname));
      if (state.field === fname) state.field = null;
      refreshEntity();
    });
    tdDel.append(delBtn);
    tr.append(tdName, tdType, tdReq, tdDel);
    tr.addEventListener("click", () => {
      state.field = fname;
      refreshEntity();
    });
    tbody.append(tr);
  }
  ftable.append(tbody);
  const addField = el("button");
  addField.textContent = "＋ 字段";
  addField.addEventListener("click", () => {
    const name = nextName(fields, "new_field");
    setPathRoot(tPath.concat("fields", name),
      { name, type: { kind: "String" }, description: null, default: null });
    state.field = name;
    refreshEntity();
  });
  const tplSel = el("select");
  tplSel.className = "tpl-select";
  tplSel.title = "常用规则模板（数值范围/枚举/外键引用）一键生成字段草稿";
  tplSel.append(option("从模板加字段…"));
  FIELD_TEMPLATES.forEach((t) => {
    const o = option(t.label);
    o.title = t.hint;
    tplSel.append(o);
  });
  tplSel.addEventListener("change", () => {
    const tpl = FIELD_TEMPLATES.find((t) => t.label === tplSel.value);
    if (!tpl) return;
    const f = tpl.make(fields);
    setPathRoot(tPath.concat("fields", f.name), f);
    state.field = f.name;
    tplSel.value = "从模板加字段…";
    refreshEntity();
  });
  const addRow = el("div", "field-actions");
  addRow.append(addField, tplSel);
  fsec.append(ftable);
  fsec.append(addRow);
  // 字段编辑器容器（选中字段时填充）
  const fed = el("div");
  fed.id = "field-editor";
  fsec.append(fed);
  box.append(fsec);

  if (state.field && fields[state.field]) renderFieldForm(fed, fields[state.field], state.field, tPath);
}

function renderFieldForm(fed, field, fname, tPath) {
  fed.replaceChildren();
  const fPath = tPath.concat("fields", fname);
  const kind = (field.type || {}).kind;

  const head = el("h3", null, `字段 · ${fname}`);
  fed.append(head);

  // —— 字段定义 ——
  const def = entitySection("字段定义");
  const nameInput = textInput(fPath.concat("name"));
  nameInput.setAttribute("data-path", fPath.concat("name").join("."));
  nameInput.addEventListener("change", () =>
    renameEntity({ kind: "field", table: state.sel.name, field: fname }, fname, nameInput));
  def.append(entityRow("名称", nameInput));
  def.append(entityRow("类型", new TypeEditor({ path: fPath.concat("type"), depth: 0 })));
  const reqWrap = el("div", "checkbox-row");
  reqWrap.append(boolInput(fPath.concat("required")), el("span", null, "必填（required）"));
  def.append(entityRow("必填", reqWrap));
  def.append(entityRow("描述", textInput(fPath.concat("description"), { toNull: true }), fPath.concat("description").join(".")));
  def.append(entityRow("默认值", jsonInput(fPath.concat("default")), fPath.concat("default").join(".")));
  fed.append(def);

  // —— 类型相关约束 ——
  const cons = entitySection("类型约束");
  if (NUMERIC_KINDS.includes(kind)) {
    cons.append(entityRow("最小值 min", numberInput(fPath.concat("min")), fPath.concat("min").join(".")));
    cons.append(entityRow("最大值 max", numberInput(fPath.concat("max")), fPath.concat("max").join(".")));
  }
  if (kind === STRING_KIND) {
    cons.append(entityRow("最小长度", intInput(fPath.concat("min_length"))));
    cons.append(entityRow("最大长度", intInput(fPath.concat("max_length"))));
    cons.append(entityRow("正则 pattern", textInput(fPath.concat("pattern"))));
    cons.append(entityRow("取值白名单（逗号分隔）", listInput(fPath.concat("enum_values"))));
  }
  if (kind === "Array") {
    cons.append(entityRow("最少元素", intInput(fPath.concat("min_items"))));
    cons.append(entityRow("最多元素", intInput(fPath.concat("max_items"))));
  }
  fed.append(cons);

  // —— 引用 ——
  const ref = entitySection("引用");
  const hasRef = !!field.reference;
  const refToggle = el("div", "checkbox-row");
  const refCheck = document.createElement("input");
  refCheck.type = "checkbox";
  refCheck.checked = hasRef;
  refCheck.addEventListener("change", () => {
    if (refCheck.checked) {
      // 默认目标表取当前表之外的第一张表（无表时留空，下拉显示占位项）。
      // REF-FIX-001：旧实现误用 enumNames()[0] 当默认表，导致下拉空白。
      setPathRoot(fPath.concat("reference"), { table: tableNames(state.sel && state.sel.name)[0] || "", field: "", cardinality: "one" });
      refreshEntity();
    } else {
      delPathRoot(fPath.concat("reference"));
      refreshEntity();
    }
  });
  refToggle.append(refCheck, el("span", null, "引用其他表字段"));
  ref.append(refToggle);
  if (hasRef) {
    const rPath = fPath.concat("reference");
    // —— 目标表下拉（REF-FIX-001）——
    // 占位项兜底空值；存量值不在表名集合（如历史 bug 写入的枚举名/已删表）
    // 时就地治愈为占位，避免浏览器把显示回退到第一项造成“所见非所存”。
    const tableSel = el("select");
    tableSel.append(option("", "（选择目标表…）"));
    Object.keys(state.doc.tables).forEach((n) => tableSel.append(option(n)));
    const storedTable = field.reference.table || "";
    tableSel.value = storedTable;
    if (tableSel.value !== storedTable) {
      field.reference.table = "";
      setPathRoot(rPath.concat("table"), "");
    }
    tableSel.setAttribute("data-path", rPath.concat("table").join("."));
    tableSel.addEventListener("change", () => setPathRoot(rPath.concat("table"), tableSel.value));
    if (!Object.keys(state.doc.tables).length) {
      const noneRow = el("div", "type-kind-hint");
      noneRow.textContent = "当前 Schema 没有可引用的表，请先创建表。";
      ref.append(entityRow("目标表", noneRow));
    } else {
      ref.append(entityRow("目标表", tableSel));
    }
    // —— 目标字段下拉：随目标表联动，只列该表字段 ——
    const fieldSel = el("select");
    const relinkFieldSel = () => {
      const target = state.doc.tables[getPath(state.doc, rPath.concat("table"))] || null;
      fieldSel.replaceChildren();
      fieldSel.append(option("", "（选择目标字段…）"));
      const stored = getPath(state.doc, rPath.concat("field")) || "";
      const known = target ? Object.keys(target.fields || {}) : [];
      known.forEach((n) => fieldSel.append(option(n)));
      fieldSel.value = stored;
      if (fieldSel.value !== stored) {
        // 目标表变了或存量字段不存在：治愈为占位，等用户选。
        setPathRoot(rPath.concat("field"), "");
      }
      fieldSel.disabled = !target;
    };
    relinkFieldSel();
    fieldSel.setAttribute("data-path", rPath.concat("field").join("."));
    fieldSel.addEventListener("change", () => setPathRoot(rPath.concat("field"), fieldSel.value));
    ref.append(entityRow("目标字段", fieldSel));
    // 表切换后联动重建目标字段选项。
    tableSel.addEventListener("change", relinkFieldSel);
    const cardSel = el("select");
    ["one", "many", "optional"].forEach((c) => cardSel.append(option(c, c === "one" ? "one（一对一）" : c === "many" ? "many（一对多）" : "optional（可选）")));
    cardSel.value = field.reference.cardinality || "one";
    cardSel.addEventListener("change", () => setPathRoot(rPath.concat("cardinality"), cardSel.value));
    ref.append(entityRow("基数", cardSel));
    const compatInput = textInput(rPath.concat("compatible_with"));
    const compatArr = field.reference.compatible_with || [];
    compatInput.value = compatArr.join(", ");
    compatInput.addEventListener("change", () => {
      const arr = compatInput.value.split(",").map((s) => s.trim()).filter(Boolean);
      if (arr.length) setPathRoot(rPath.concat("compatible_with"), arr);
      else delPathRoot(rPath.concat("compatible_with"));
    });
    ref.append(entityRow("兼容 profile（逗号）", compatInput));
  }
  fed.append(ref);

  // —— 可见性与规则 ——
  const vis = entitySection("可见性与规则");
  vis.append(entityRow("Targets（profile 可见性）", listInput(fPath.concat("targets"))));
  renderRulesEditor(vis, fPath, field);
  fed.append(vis);

  // —— 自定义元数据（flatten 键） ——
  const meta = entitySection("自定义元数据（JSON 对象，额外键将原样写入并保留）");
  const metaBox = el("div");
  const metaInput = el("textarea");
  const extras = {};
  for (const [k, v] of Object.entries(field)) {
    if (!FIELD_KEYS.includes(k)) extras[k] = v;
  }
  metaInput.value = JSON.stringify(extras, null, 2);
  metaInput.placeholder = "{ \"custom_note\": \"keep\" }";
  metaInput.addEventListener("change", () => {
    const raw = metaInput.value.trim();
    if (!raw) {
      for (const k of Object.keys(extras)) delPathRoot(fPath.concat(k));
      metaInput.value = "";
      return;
    }
    try {
      const parsed = JSON.parse(raw);
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
        toast("自定义元数据必须是 JSON 对象", "err");
        return;
      }
      for (const k of Object.keys(extras)) delPathRoot(fPath.concat(k));
      for (const [k, v] of Object.entries(parsed)) {
        if (!FIELD_KEYS.includes(k)) setPathRoot(fPath.concat(k), v);
        else toast(`键 ${k} 是预留键，忽略`, "err");
      }
      metaInput.classList.remove("json-bad");
    } catch {
      metaInput.classList.add("json-bad");
      toast("自定义元数据 JSON 无法解析，未写入", "err");
    }
    refreshEntity();
  });
  metaBox.append(metaInput);
  meta.append(metaBox);
  fed.append(meta);
}

// ── 规则人话层：正反例（P1-4）──────────────────────────────────────────
//
// 样例行（JSON 对象）+ 期望结果 + 备注。「试例」把规则连同正反例 POST
// /api/rule/examples，由校验主链同一套表达式引擎试算，逐例回报「期望 vs
// 实际」——策划当场确认规则读起来是不是这个意思。试算只是提示，不改文档。
const EXAMPLE_EXPECTED = ["pass", "violate"];

/// 路径绑定的下拉（人话层的期望结果用；dom.js 的输入只覆盖文本/数字/列表）。
function selectInput(path, values) {
  const sel = el("select");
  for (const v of values) sel.append(option(v, v));
  const cur = getPath(state.doc, path);
  sel.value = values.includes(cur) ? cur : values[0];
  sel.addEventListener("change", () => setPathRoot(path, sel.value));
  return sel;
}

function renderRuleExamples(rPath, rule) {
  const boxPath = rPath.concat("examples");
  const examples = Array.isArray(rule && rule.examples) ? rule.examples : [];
  const box = el("div", "nested-list");
  box.append(el("span", "type-tag", "正反例"));

  examples.forEach((_, i) => {
    const ePath = boxPath.concat(String(i));
    const row = el("div", "nested-row");
    const g = el("div", "nested-grid cols-4");
    g.append(el("label", null, "样例行"),
      jsonInput(ePath.concat("row")),
      el("label", null, "期望"),
      selectInput(ePath.concat("expected"), EXAMPLE_EXPECTED));
    const del = el("button", "icon danger small", "删除");
    del.addEventListener("click", () => {
      const items = getPath(state.doc, boxPath) || [];
      setPathRoot(boxPath, items.filter((_, j) => j !== i));
      refreshEntity();
    });
    g.append(el("label", null, ""), del, el("label", null, ""), el("label", null, ""));
    const g2 = el("div", "nested-grid cols-4");
    g2.append(el("label", null, "备注"), textInput(ePath.concat("note"), { toNull: true }));
    row.append(g, g2);
    box.append(row);
  });

  const add = el("button", "", "＋ 加正反例");
  add.addEventListener("click", () => {
    const items = getPath(state.doc, boxPath) || [];
    setPathRoot(boxPath, [...items, { row: {}, expected: "pass" }]);
    refreshEntity();
  });

  const tryBtn = el("button", "", "试例");
  const summary = el("div", "type-kind-hint");
  const detail = el("div", "nested-list");
  tryBtn.addEventListener("click", async () => {
    detail.replaceChildren();
    const r = getPath(state.doc, rPath);
    if (!r) return;
    const res = await api("/api/rule/examples", JSON.stringify({ rule: r }));
    if (!res.data.ok) { summary.textContent = `试算失败：${res.data.error}`; return; }
    const checks = res.data.checks || [];
    const good = checks.filter((c) => c.ok).length;
    summary.textContent = checks.length
      ? `试算：${good}/${checks.length} 例符合期望`
      : "还没有正反例可试（先「＋ 加正反例」）";
    for (const [i, c] of checks.entries()) {
      detail.append(el("span", null,
        `${c.ok ? "✔" : "✘"} 例 ${i + 1}：期望 ${c.expected}，实际 ${c.actual}`
        + (c.note ? `（${c.note}）` : "")));
    }
  });

  const btnRow = el("div", "nested-row");
  btnRow.append(tryBtn);
  box.append(add, btnRow, summary, detail);
  return box;
}

// ── 表达式规则编辑器（基础模式字段表单 / 规则模式共用）────────────────
//
// 唯一的规则表示：`field.rules`（ExpressionRule 数组，cage-core 语义）。
// 基础、规则、专家三模式读写同一数组——不存在第二套规则格式（P1-1）。
// 可视化条件构建器（P1-2）与九项规则模板（P1-3）在此层继续扩展。
function renderRulesEditor(fed, fPath, field) {
  const rules = el("div", "nested-list");
  const rulesArr = field.rules || [];
  rulesArr.forEach((_, i) => {
    const row = el("div", "nested-row");
    const grip = el("div", "nested-grip");
    grip.append(el("span", null, `规则 #${i}`));
    const delBtn = el("button", "icon danger small", "删除");
    delBtn.addEventListener("click", () => {
      delPathRoot(fPath.concat("rules", String(i)));
      refreshEntity();
    });
    grip.append(delBtn);
    row.append(grip);
    const grid = el("div", "nested-grid cols-4");
    const rPath = fPath.concat("rules", String(i));
    grid.append(el("label", null, "规则名"),
      textInput(rPath.concat("name")),
      el("label", null, "断言表达式"),
      textInput(rPath.concat("assert")));
    row.append(grid);
    const grid2 = el("div", "nested-grid cols-4");
    grid2.append(el("label", null, "提示消息"),
      textInput(rPath.concat("message"), { toNull: true }),
      el("label", null, "仅警告"),
      (() => {
        const w = el("div", "checkbox-row");
        w.append(boolInput(rPath.concat("warning_only")));
        return w;
      })());
    row.append(grid2);
    // ── 规则人话层（P1-4）：中文标题 / 业务说明 ──
    const grid3 = el("div", "nested-grid cols-4");
    grid3.append(el("label", null, "中文标题"),
      textInput(rPath.concat("title"), { toNull: true }),
      el("label", null, "业务说明"),
      textInput(rPath.concat("description"), { toNull: true }));
    row.append(grid3);
    // ── 正反例（P1-4）：样例行 + 期望结果 + 备注，可一键试算 ──
    row.append(renderRuleExamples(rPath, rulesArr[i]));
    rules.append(row);
  });
  const addRule = el("button");
  addRule.textContent = "＋ 表达式规则";
  addRule.addEventListener("click", () => {
    const items = field.rules || [];
    setPathRoot(fPath.concat("rules"), [...items, { name: `rule_${items.length + 1}`, assert: "" }]);
    refreshEntity();
  });
  fed.append(rules, addRule);
}

// ── 表达能力边界提示（P1-5）────────────────────────────────────────────
//
// 引擎表达不了时（构建器编译被拒 / 模板不可表达），都用这一份醒目的
// 边界提示：讲清限制（不夸大）、给出两条诚实出路（九项规则模板 / Python
// 插件 P2-2 或专门 Rust 校验器 P3-2）。绝不在提示里假装能自动转换。
function renderBoundaryNotice(reason) {
  const g = boundaryGuidance(reason);
  const box = el("div", "type-kind-hint");
  box.append(el("div", null, `⚠ ${g.heading}`));
  const list = el("div", "nested-list");
  for (const limit of g.limits) list.append(el("span", null, `· ${limit}`));
  for (const route of g.routes) list.append(el("span", null, `→ ${route}`));
  box.append(list);
  return box;
}

// ── 可视化条件构建器（P1-2）────────────────────────────────────────────
//
// 行模型 → AST（modes.js builderRowsToAst）→ Rust Core 编译
// （POST /api/rule/compile）→ 该字段的 `field.rules`（ExpressionRule
// 数组）。构建器只产出 AST，前端从不手写 assert 文本；既有 rules 经
// POST /api/rule/decompile 反向装载回行。表达式引擎只支持单条件比较
// （field OP 字面量/字段），无法表达的 AST 以边界提示拒绝（P1-5 使其
// 更醒目），绝不静默生成规则。
const RULE_OPS = ["==", "!=", "<", "<=", ">", ">="];
// 构建器草稿（含行与元数据）随重渲染保留；提交成功后清除。
const BUILDER_DRAFTS = new Map();
const builderKey = (table, field) => `${table}.${field}`;

function loadDraft(table, field, count) {
  return BUILDER_DRAFTS.get(builderKey(table, field))
    || {
      rows: [{ field, op: "==", rhsKind: "Literal", value: "", fieldName: "" }],
      name: `rule_${count + 1}`, message: "", warningOnly: false,
      title: "", description: "",
    };
}

// 行 → 可编译 AST；字面量经 parseLiteralInput（`200`→200，`hi`→"hi"）。
function rowsToAst(rows) {
  const items = [];
  for (const r of rows) {
    let rhs;
    if (r.rhsKind === "Field") {
      if (!r.fieldName) return { error: `第 ${items.length + 1} 行：缺比较字段名` };
      rhs = { kind: "Field", name: r.fieldName };
    } else {
      const p = parseLiteralInput(r.value);
      if (p.error) return { error: `第 ${items.length + 1} 行：${p.error}` };
      rhs = { kind: "Literal", value: p.value };
    }
    items.push({ kind: "Compare", field: r.field, op: r.op, rhs });
  }
  return { ast: builderRowsToAst(items) };
}

function renderConditionBuilder(fed, tName, table, fPath, field) {
  const fname = state.field;
  const fields = table.fields || {};
  const draft = loadDraft(tName, fname, (field.rules || []).length);
  const note = el("div", "type-kind-hint");
  note.textContent =
    "表达能力边界：规则引擎当前只支持单条件比较（字段 OP 字面量/字段），"
    + "无 AND/OR、无数学运算。本构建器只生成它能表达的 AST，无法表达的以"
    + "边界提示拒绝。多行条件同时成立（编译后逐行成规则）。";
  fed.append(note);

  const sec = el("div", "nested-list");
  const rowsBox = el("div");
  sec.append(rowsBox);

  const renderRows = () => {
    rowsBox.replaceChildren();
    draft.rows.forEach((r, i) => {
      const row = el("div", "nested-row");
      const grid = el("div", "nested-grid cols-4");

      const fsel = el("select");
      for (const fn of Object.keys(fields)) fsel.append(option(fn, fn));
      fsel.value = r.field || fname;
      fsel.addEventListener("change", () => { r.field = fsel.value; refreshEntity(); });

      const osel = el("select");
      for (const op of RULE_OPS) osel.append(option(op, op));
      osel.value = r.op;
      osel.addEventListener("change", () => { r.op = osel.value; refreshEntity(); });

      const kindSel = el("select");
      kindSel.append(option("Literal", "字面量"), option("Field", "字段"));
      kindSel.value = r.rhsKind;
      kindSel.addEventListener("change", () => { r.rhsKind = kindSel.value; refreshEntity(); });

      const del = el("button", "icon danger small", "×");
      del.addEventListener("click", () => {
        draft.rows.splice(i, 1);
        if (!draft.rows.length) draft.rows.push({ field: fname, op: "==", rhsKind: "Literal", value: "", fieldName: "" });
        BUILDER_DRAFTS.set(builderKey(tName, fname), draft);
        refreshEntity();
      });

      grid.append(el("label", null, "字段"), fsel, el("label", null, "操作"), osel);
      grid.append(el("label", null, "右侧"), kindSel, el("label", null, ""), del);

      const valBox = el("div", "nested-grid cols-4");
      if (r.rhsKind === "Literal") {
        valBox.append(el("label", null, "字面量"),
          (() => {
            const inp = el("input");
            inp.value = r.value;
            inp.placeholder = "200 / \"hi\" / true";
            inp.addEventListener("change", () => { r.value = inp.value; refreshEntity(); });
            return inp;
          })(),
          el("label", null, ""), el("label", null, ""));
      } else {
        const f2 = el("select");
        for (const fn of Object.keys(fields)) f2.append(option(fn, fn));
        f2.value = r.fieldName || "";
        f2.addEventListener("change", () => { r.fieldName = f2.value; refreshEntity(); });
        valBox.append(el("label", null, "比较字段"), f2, el("label", null, ""), el("label", null, ""));
      }
      row.append(grid, valBox);
      rowsBox.append(row);
    });
  };
  renderRows();

  const addBtn = el("button", "", "＋ 添加条件");
  addBtn.addEventListener("click", () => {
    draft.rows.push({ field: fname, op: "==", rhsKind: "Literal", value: "", fieldName: "" });
    BUILDER_DRAFTS.set(builderKey(tName, fname), draft);
    refreshEntity();
  });
  fed.append(sec, addBtn);

  // 规则元数据（名/消息/仅警告）与写入方式。
  const metaGrid = el("div", "nested-grid cols-4");
  const nameInp = el("input");
  nameInp.value = draft.name;
  nameInp.addEventListener("change", () => { draft.name = nameInp.value; });
  const msgInp = el("input");
  msgInp.value = draft.message;
  msgInp.placeholder = "提示消息（可空）";
  msgInp.addEventListener("change", () => { draft.message = msgInp.value; });
  const warnChk = el("input");
  warnChk.type = "checkbox";
  warnChk.checked = draft.warningOnly;
  warnChk.addEventListener("change", () => { draft.warningOnly = warnChk.checked; });
  const titleInp = el("input");
  titleInp.value = draft.title;
  titleInp.placeholder = "中文标题（人话层，P1-4）";
  titleInp.addEventListener("change", () => { draft.title = titleInp.value; });
  const descInp = el("input");
  descInp.value = draft.description;
  descInp.placeholder = "业务说明（人话层，P1-4）";
  descInp.addEventListener("change", () => { draft.description = descInp.value; });
  metaGrid.append(el("label", null, "规则名"), nameInp, el("label", null, "提示消息"), msgInp);
  metaGrid.append(el("label", null, "仅警告"), warnChk, el("label", null, "中文标题"), titleInp);
  metaGrid.append(el("label", null, "业务说明"), descInp, el("label", null, ""), el("label", null, ""));

  const writeSel = el("select");
  writeSel.append(option("replace", "替换该字段全部规则"), option("append", "追加到现有规则"));
  const boundaryBox = el("div");
  boundaryBox.hidden = true;
  metaGrid.append(el("label", null, "写入方式"), writeSel, el("label", null, ""), el("label", null, ""));
  fed.append(metaGrid, boundaryBox);

  const compileBtn = el("button", "primary", "编译并写入 rules");
  compileBtn.addEventListener("click", async () => {
    const realRows = draft.rows.filter((r) => r.field);
    if (!realRows.length) return toast("请先填写至少一个条件", "error");
    const { ast, error } = rowsToAst(realRows);
    if (error) return toast(error, "error");
    const meta = {
      name: draft.name || `rule_${(field.rules || []).length + 1}`,
      message: draft.message || null,
      warning_only: draft.warningOnly,
      title: draft.title || null,
      description: draft.description || null,
    };
    const res = await api("/api/rule/compile", JSON.stringify({ ast, meta }));
    if (!res.data.ok) {
      // 引擎表达不了：醒目的边界提示（讲清限制 + 两条诚实出路），
      // 不假装能自动转换（P1-5）。
      boundaryBox.replaceChildren(renderBoundaryNotice(res.data.error));
      boundaryBox.hidden = false;
      toast(`无法编译：${res.data.error}`, "error");
      return;
    }
    const rules = res.data.rules;
    const mode = writeSel.value;
    // 替换时把既有规则的人话层（标题/说明/正反例）带回同样 assert 的
    // 新规则上——一次无关的重编译不丢策划填写的说明（P1-4）。
    const written = mode === "replace"
      ? mergeHumanLayer(rules, field.rules || [])
      : [...(field.rules || []), ...rules];
    setPathRoot(fPath.concat("rules"), written);
    BUILDER_DRAFTS.delete(builderKey(tName, fname));
    markDirty();
    refreshEntity();
    toast(`已编译 ${rules.length} 条规则（${mode === "replace" ? "替换" : "追加"}）`);
  });

  const loadBtn = el("button", "", "从既有规则装载");
  loadBtn.addEventListener("click", async () => {
    const rules = field.rules || [];
    if (!rules.length) return toast("该字段还没有规则", "error");
    const rows = [];
    let unparseable = 0;
    for (const r of rules) {
      const res = await api("/api/rule/decompile", JSON.stringify({ rule: r }));
      if (!res.data.ok) continue;
      const ast = res.data.ast;
      const push = (node) => {
        if (node.kind === "Compare") {
          rows.push({
            field: node.field,
            op: node.op,
            rhsKind: node.rhs.kind,
            value: node.rhs.kind === "Literal" ? node.rhs.value : "",
            fieldName: node.rhs.kind === "Field" ? node.rhs.name : "",
          });
        } else if (node.kind === "And") {
          node.items.forEach(push);
        } else {
          unparseable += 1;
        }
      };
      push(ast);
    }
    if (!rows.length) return toast("既有规则都无法表达为可视化条件", "error");
    if (unparseable) toast(`${unparseable} 条规则无法可视化，已跳过`, "error");
    BUILDER_DRAFTS.set(builderKey(tName, fname), {
      rows,
      name: rules[0].name || `rule_${rules.length}`,
      message: rules[0].message || "",
      warningOnly: Boolean(rules[0].warning_only),
      // 人话层随规则带上，替换重编译不丢（P1-4）。
      title: rules[0].title || "",
      description: rules[0].description || "",
    });
    refreshEntity();
  });
  const btnRow = el("div", "nested-row");
  btnRow.append(compileBtn, loadBtn);
  fed.append(btnRow);
}

// ── 规则模式（P1-1）────────────────────────────────────────────────────
//
// 规则这一层的专用视图，仍编辑同一份 `state.doc` 的 `field.rules`——与
// 基础模式（字段表单里的规则编辑）和专家模式（原始文档）写同一数组，
// 不引入第二套规则格式。可视化条件构建器（P1-2）与九项模板（P1-3）在
// 此层接入。
function renderRulesMode() {
  const box = document.getElementById("entity");
  box.replaceChildren();
  box.append(el("h2", null, "规则模式 · 表达式规则（同一份 Schema 语义模型）"));
  const note = el("div", "type-kind-hint");
  note.textContent =
    "规则挂在字段的 rules 数组（ExpressionRule）。基础、专家模式编辑同一数组——"
    + "不存在第二套规则格式。可视化条件构建器（P1-2）与九项规则模板（P1-3）在此层接入。";
  box.append(note);

  const kind = state.sel && state.sel.kind;
  if (kind === "enum") {
    box.append(el("div", "type-kind-hint", "枚举取值不挂表达式规则；请选择表字段。"));
    return;
  }
  if (!kind) {
    box.append(el("div", "type-kind-hint", "请选择表（或表内字段）以编辑其规则。"));
    return;
  }
  const table = state.doc.tables[state.sel.name];
  if (!table) return;
  const tPath = ["tables", state.sel.name];
  const fields = table.fields || {};

  // 选中了字段：直接编辑该字段的规则。
  if (state.field && fields[state.field]) {
    const fed = el("div");
    const fPath = tPath.concat("fields", state.field);
    fed.append(el("h3", null, `字段 · ${state.field} · rules`));
    renderConditionBuilder(fed, state.sel.name, table, fPath, fields[state.field]);
    const editorWrap = el("div");
    editorWrap.append(el("h4", null, "表达式规则（唯一表示）"));
    renderRulesEditor(editorWrap, fPath, fields[state.field]);
    fed.append(editorWrap);
    renderRuleTemplates(fed, state.sel.name, state.field);
    box.append(fed);
    return;
  }

  // 未选中字段：表级概览 + 各字段规则入口。
  const fsec = entitySection("字段规则");
  const list = el("div", "nested-list");
  for (const [fname, f] of Object.entries(fields)) {
    const row = el("div", "nested-row");
    const grip = el("div", "nested-grip");
    grip.append(el("span", null, fname));
    grip.append(el("span", "type-tag", `${(f.rules || []).length} 规则`));
    const btn = el("button", "", "编辑规则…");
    btn.addEventListener("click", () => {
      state.field = fname;
      refreshEntity();
    });
    grip.append(btn);
    row.append(grip);
    list.append(row);
  }
  fsec.append(list);
  box.append(fsec);
  renderRuleTemplates(box, state.sel.name, "");
}

// ── 内置规则模板九项（P1-3）────────────────────────────────────────────
//
// 规划逻辑是纯函数（rule_templates.js，node 可测）：可表达的模板产出
// Schema 补丁或 AST（字段比较仍经 /api/rule/compile 编译进唯一的
// `field.rules`），表达不了的四项返回 boundary 引导（P1-5 醒目呈现），
// 绝不假装自动转换。DOM 层只收集参数并应用。
function renderRuleTemplates(box, tName, fieldName) {
  const sec = entitySection(`规则模板（九项）· ${fieldName || tName}`);
  const fields = (state.doc.tables[tName] || {}).fields || {};

  const tplSel = el("select");
  tplSel.append(option("", "选择规则模板…"));
  RULE_TEMPLATES.forEach((t) => {
    const o = option(t.id, `${t.name}（${t.engine}）`);
    o.title = t.expressible ? t.hint : t.guidance;
    tplSel.append(o);
  });
  const paramBox = el("div", "nested-grid cols-4");
  const note = el("div", "type-kind-hint");
  note.hidden = true;
  const applyBtn = el("button", "primary", "应用模板");
  applyBtn.disabled = true;

  const fieldSel = el("select");
  if (!fieldName) {
    fieldSel.append(option("", "选择字段…"));
    for (const fn of Object.keys(fields)) fieldSel.append(option(fn, fn));
  }

  const refresh = () => {
    paramBox.replaceChildren();
    note.replaceChildren();
    note.hidden = true;
    applyBtn.disabled = true;
    const tpl = templateById(tplSel.value);
    if (!tpl) return;
    if (!tpl.expressible) {
      note.replaceChildren(renderBoundaryNotice(tpl.guidance));
      note.hidden = false;
      return;
    }
    if (!fieldName && !fieldSel.value) {
      note.replaceChildren(el("span", null, "该模板作用于字段：请先选择字段。"));
      note.hidden = false;
      return;
    }
    applyBtn.disabled = false;
    switch (tpl.id) {
      case "primary_key":
        paramBox.append(el("label", null, "字段"), el("span", "type-tag", fieldName || fieldSel.value),
          el("label", null, ""), el("label", null, ""));
        break;
      case "range": {
        const min = el("input"); min.type = "number"; min.placeholder = "min";
        const max = el("input"); max.type = "number"; max.placeholder = "max";
        min.dataset.tpl = "min"; max.dataset.tpl = "max";
        paramBox.append(el("label", null, "最小值"), min, el("label", null, "最大值"), max);
        break;
      }
      case "enum": {
        const es = el("select");
        for (const en of enumNames()) es.append(option(en, en));
        es.dataset.tpl = "enumName";
        paramBox.append(el("label", null, "枚举"), es, el("label", null, ""), el("label", null, ""));
        break;
      }
      case "reference": {
        const ts = el("select");
        for (const tn of tableNames()) ts.append(option(tn, tn));
        ts.dataset.tpl = "refTable";
        const fs = el("select");
        const fillRefFields = (tn) => {
          fs.replaceChildren();
          const rf = (state.doc.tables[tn] || {}).fields || {};
          for (const fn of Object.keys(rf)) fs.append(option(fn, fn));
        };
        fillRefFields(ts.value);
        ts.addEventListener("change", () => fillRefFields(ts.value));
        fs.dataset.tpl = "refField";
        paramBox.append(el("label", null, "目标表"), ts, el("label", null, "目标字段"), fs);
        break;
      }
      case "field_compare": {
        const ops = el("select");
        for (const op of ["==", "!=", "<", "<=", ">", ">="]) ops.append(option(op, op));
        ops.dataset.tpl = "op";
        const os = el("select");
        for (const fn of Object.keys(fields)) {
          if (fn !== (fieldName || fieldSel.value)) os.append(option(fn, fn));
        }
        os.dataset.tpl = "otherField";
        paramBox.append(el("label", null, "操作"), ops, el("label", null, "另一字段"), os);
        break;
      }
    }
  };
  tplSel.addEventListener("change", refresh);
  fieldSel.addEventListener("change", refresh);

  applyBtn.addEventListener("click", async () => {
    const params = { table: tName, field: fieldName || fieldSel.value };
    paramBox.querySelectorAll("[data-tpl]").forEach((n) => {
      params[n.dataset.tpl] = n.value;
    });
    const plan = planTemplate(tplSel.value, params, state.doc);
    if (plan.kind === "error") return toast(plan.error, "error");
    if (plan.kind === "boundary") {
      note.replaceChildren(renderBoundaryNotice(plan.guidance));
      note.hidden = false;
      toast("该模板超出当前引擎表达能力，见引导说明", "error");
      return;
    }
    if (plan.kind === "patches") {
      for (const p of plan.patches) setPathRoot(p.path, p.value);
      markDirty();
      refreshEntity();
      toast(`模板已应用：${plan.summary}`);
      return;
    }
    // 字段比较：AST 交给 Rust Core 编译（前端不写 assert）。
    const res = await api("/api/rule/compile", JSON.stringify({ ast: plan.ast, meta: plan.meta }));
    if (!res.data.ok) {
      note.replaceChildren(renderBoundaryNotice(res.data.error));
      note.hidden = false;
      toast(res.data.error || "编译失败", "error");
      return;
    }
    const rulesPath = ["tables", tName, "fields", params.field, "rules"];
    const existing = getPath(state.doc, rulesPath) || [];
    setPathRoot(rulesPath, [...existing, ...res.data.rules]);
    markDirty();
    refreshEntity();
    toast(`模板已编译 ${res.data.rules.length} 条规则：${plan.summary}`);
  });

  const head = el("div", "nested-row");
  if (!fieldName) head.append(el("label", null, "字段"), fieldSel);
  head.append(el("label", null, "模板"), tplSel, applyBtn);
  sec.append(head, paramBox, note);
  box.append(sec);
}

// 枚举编辑器
function renderEnumForm() {
  const box = document.getElementById("entity");
  box.replaceChildren();
  const enm = state.doc.enums[state.sel.name];
  if (!enm) return;

  const ePath = ["enums", state.sel.name];
  const title = el("h2", null, `枚举 · ${state.sel.name}`);
  box.append(title);

  const def = entitySection("枚举定义");
  const nameInput = textInput(ePath.concat("name"));
  nameInput.setAttribute("data-path", ePath.concat("name").join("."));
  nameInput.addEventListener("change", () =>
    renameEntity({ kind: "enum", name: state.sel.name }, state.sel.name, nameInput));
  def.append(entityRow("名称", nameInput));
  def.append(entityRow("描述", textInput(ePath.concat("description"), { toNull: true })));
  box.append(def);

  const vals = entitySection("取值");
  const list = el("div", "nested-list");
  list.setAttribute("data-path", ePath.concat("values").join("."));
  const values = enm.values || [];
  const renderVals = () => {
    list.replaceChildren();
    values.forEach((_, i) => {
      const row = el("div", "nested-row");
      const grip = el("div", "nested-grip");
      grip.append(el("span", null, `#${i}`));
      const delBtn = el("button", "icon danger small", "删除");
      delBtn.addEventListener("click", () => {
        delPathRoot(ePath.concat("values", String(i)));
        refreshEntity();
      });
      grip.append(delBtn);
      row.append(grip);
      const grid = el("div", "nested-grid cols-3");
      const vPath = ePath.concat("values", String(i));
      grid.append(el("label", null, "名称"), textInput(vPath.concat("name")));
      grid.append(el("label", null, "字面值"), jsonInput(vPath.concat("value")));
      const grid2 = el("div", "nested-grid cols-3");
      grid2.append(el("label", null, "描述"), textInput(vPath.concat("description"), { toNull: true }));
      row.append(grid, grid2);
      list.append(row);
    });
  };
  renderVals();
  const addVal = el("button");
  addVal.textContent = "＋ 取值";
  addVal.addEventListener("click", () => {
    const taken = {};
    values.forEach((v, i) => { taken[v.name || `v${i}`] = true; });
    setPathRoot(ePath.concat("values"), [...values, { name: nextName(taken, "new_value"), value: null, description: null }]);
    refreshEntity();
  });
  vals.append(list, addVal);
  box.append(vals);
}

// ── 树（左栏） ──────────────────────────────────────────────────────

function renderTree() {
  const tList = document.getElementById("table-list");
  const eList = document.getElementById("enum-list");
  tList.replaceChildren();
  eList.replaceChildren();
  for (const [name, table] of Object.entries(state.doc.tables || {})) {
    const li = el("li");
    const item = el("div", "tree-item");
    item.classList.toggle("active", state.sel && state.sel.kind === "table" && state.sel.name === name);
    item.textContent = name;
    const tableHidden = state.scope && state.scope.hiddenTables.has(name);
    if (tableHidden) {
      item.classList.add("scoped-hidden");
      item.append(el("span", "scope-badge", "本 profile 不可见"));
    }
    const tChips = targetsChips(table.targets);
    if (tChips) item.append(tChips);
    item.append(el("span", "type-tag", `${Object.keys(table.fields || {}).length} 字段`));
    item.addEventListener("click", () => selectEntity("table", name));
    li.append(item);
    tList.append(li);
  }
  for (const [name, enm] of Object.entries(state.doc.enums || {})) {
    const li = el("li");
    const item = el("div", "tree-item");
    item.classList.toggle("active", state.sel && state.sel.kind === "enum" && state.sel.name === name);
    item.textContent = name;
    item.append(el("span", "type-tag", `${(enm.values || []).length} 取值`));
    item.addEventListener("click", () => selectEntity("enum", name));
    li.append(item);
    eList.append(li);
  }
}

function selectEntity(kind, name) {
  state.sel = { kind, name };
  state.field = null;
  refreshEntity();
}

function refreshEntity() {
  renderTree();
  const entity = document.getElementById("entity");
  const hint = document.getElementById("empty-hint");
  // 编辑模式（P1-1）：专家模式渲染整份文档（无需选中实体）；规则模式
  // 渲染规则视图。两者都读写同一份 state.doc——三层共享一个语义模型。
  if (state.mode === "expert") {
    entity.hidden = false;
    hint.hidden = true;
    renderExpertMode();
    return;
  }
  if (state.mode === "rules") {
    entity.hidden = false;
    hint.hidden = true;
    renderRulesMode();
    return;
  }
  if (!state.sel) {
    entity.hidden = true;
    hint.hidden = false;
    return;
  }
  entity.hidden = false;
  hint.hidden = true;
  if (state.sel.kind === "table") renderTableForm();
  else renderEnumForm();
}

// 模板库（策划界面 P2）：前端静态字典，生成 schema 片段后走既有
// setPathRoot → markDirty → 自动重校验 → 保存链，服务端零新面。
const FIELD_TEMPLATES = [
  {
    id: "num-range", label: "数值范围",
    hint: "Int + min/max（改数值与说明）",
    make: (names) => ({
      name: nextName(names, "num_range"), type: { kind: "Int" },
      description: "数值范围字段（模板：请改 min/max）", default: null, min: 0, max: 100,
    }),
  },
  {
    id: "enum-field", label: "枚举",
    hint: "Enum 引用既有枚举（改目标枚举）",
    make: (names) => ({
      name: nextName(names, "enum_field"), type: { kind: "Enum", value: enumNames()[0] || "" },
      description: "枚举字段（模板：请改目标枚举）", default: null,
    }),
  },
  {
    id: "foreign-key", label: "外键引用",
    hint: "reference 引用其他表（改目标表/字段）",
    make: (names) => ({
      name: nextName(names, "fk_field"), type: { kind: "String" },
      description: "外键引用字段（模板：请改目标表/字段）", default: null,
      // REF-FIX-001：默认取当前表之外的第一张表，避免一键自引用。
      reference: { table: tableNames(state.sel && state.sel.name)[0] || "", field: "", cardinality: "one" },
    }),
  },
];

export { FIELD_TEMPLATES, entitySection, entityRow, renderTableForm, renderFieldForm, renderEnumForm, renderRulesMode, renderRulesEditor, renderTree, selectEntity, refreshEntity };
