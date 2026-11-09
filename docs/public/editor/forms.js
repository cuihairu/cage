// 表单渲染：表/字段/枚举/树 与模板库（split out of app.js, LAY-027）。

import { FIELD_KEYS, NUMERIC_KINDS, STRING_KIND } from "./constants.js";
import { enumNames, state } from "./state.js";
import { boolInput, delPathRoot, el, getPath, intInput, jsonInput, listInput, nextName, numberInput, option, renameKey, setPathRoot, targetsChips, textInput, toast } from "./dom.js";
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
  nameInput.addEventListener("change", () => {
    const target = nameInput.value.trim();
    const next = renameKey(state.doc.tables, state.sel.name, target);
    if (next) {
      state.doc.tables = next;
      state.sel = { kind: "table", name: target };
      refreshEntity();
    } else nameInput.value = state.sel.name;
  });
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
  nameInput.addEventListener("change", () => {
    const target = nameInput.value.trim();
    const next = renameKey(tableFieldsAt(tPath), fname, target);
    if (next) {
      setPathRoot(tPath.concat("fields"), next);
      state.field = target;
      refreshEntity();
    } else nameInput.value = fname;
  });
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
      setPathRoot(fPath.concat("reference"), { table: enumNames()[0] || "", field: "", cardinality: "one" });
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
    const tableSel = el("select");
    Object.keys(state.doc.tables).forEach((n) => tableSel.append(option(n)));
    tableSel.value = field.reference.table || "";
    tableSel.setAttribute("data-path", rPath.concat("table").join("."));
    tableSel.addEventListener("change", () => setPathRoot(rPath.concat("table"), tableSel.value));
    ref.append(entityRow("目标表", tableSel));
    ref.append(entityRow("目标字段", textInput(rPath.concat("field")), rPath.concat("field").join(".")));
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
    rules.append(row);
  });
  const addRule = el("button");
  addRule.textContent = "＋ 表达式规则";
  addRule.addEventListener("click", () => {
    const items = field.rules || [];
    setPathRoot(fPath.concat("rules"), [...items, { name: `rule_${items.length + 1}`, assert: "" }]);
    refreshEntity();
  });
  vis.append(rules, addRule);
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

function tableFieldsAt(tPath) {
  return getPath(state.doc, tPath.concat("fields")) || {};
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
  nameInput.addEventListener("change", () => {
    const target = nameInput.value.trim();
    const next = renameKey(state.doc.enums, state.sel.name, target);
    if (next) {
      state.doc.enums = next;
      state.sel = { kind: "enum", name: target };
      refreshEntity();
    } else nameInput.value = state.sel.name;
  });
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
      reference: { table: Object.keys(state.doc.tables)[0] || "", field: "", cardinality: "one" },
    }),
  },
];

export { FIELD_TEMPLATES, entitySection, entityRow, renderTableForm, renderFieldForm, tableFieldsAt, renderEnumForm, renderTree, selectEntity, refreshEntity };
