// cage Schema 编辑器 —— vanilla ES module，零依赖。
//
// 编辑器文档 = GET /api/schema 返回的 canonical serde 形状（与 snapshot
// schema.json 同一文档）。所有改动在浏览器内对文档做外科手术式修改
// （未知/遗留键原样保留），保存时整份 JSON 交给 POST /api/schema，
// 由 cage-core 反序列化、校验、以 canonical YAML 写回。前端不解析、
// 不渲染 YAML。

"use strict";

// ── 常量：19 种字段类型 + 各实体已知键 ──────────────────────────────

const KINDS = [
  "Null", "Bool",
  "Int8", "Int16", "Int32", "Int64",
  "UInt8", "UInt16", "UInt32", "UInt64",
  "Float32", "Float64",
  "String", "Bytes",
  "Array", "Object", "Map", "Enum", "Any",
];

const NUMERIC_KINDS = KINDS.slice(2, 14); // Int8..Float64
const STRING_KIND = "String";
const MAX_DEPTH = 5;

// 实体对象的已知键（用于自定义元数据识别 & 表单渲染的键序）
const TABLE_KEYS = ["name", "description", "primary_key", "fields", "unique_constraints", "order_by", "targets"];
const FIELD_KEYS = [
  "name", "type", "description", "required", "default",
  "min", "max", "min_length", "max_length", "pattern", "enum_values",
  "min_items", "max_items", "reference", "targets", "rules",
  // 遗留（保留不编辑）：items, properties, additional_properties
];

// ── 状态 ────────────────────────────────────────────────────────────

const state = {
  meta: null,        // GET /api/schema 负荷
  doc: null,         // 编辑器文档（canonical serde 形状），原地修改
  sel: null,         // {kind: "table" | "enum", name}
  field: null,       // 当前表内选中的字段名
  diags: [],         // 最近一次校验结果
  dirty: false,
};

const enumNames = () => (state.doc ? Object.keys(state.doc.enums || {}) : []);

// ── 基础工具 ────────────────────────────────────────────────────────

function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
}

function option(value, label) {
  const o = document.createElement("option");
  o.value = value;
  o.textContent = label !== undefined ? label : value;
  return o;
}

async function api(path, body) {
  const opt = body === undefined
    ? {}
    : { method: "POST", headers: { "Content-Type": "application/json" }, body };
  let res;
  try {
    res = await fetch(path, opt);
  } catch (e) {
    return { status: 0, data: { ok: false, error: `无法连接 ${path}: ${e}` } };
  }
  let data;
  try {
    data = await res.json();
  } catch {
    data = { ok: false, error: `HTTP ${res.status}：响应不是 JSON` };
  }
  return { status: res.status, data };
}

function markDirty() {
  if (!state.dirty) {
    state.dirty = true;
    const flag = document.getElementById("dirty-flag");
    flag.hidden = false;
  }
}

function toast(msg, kind, ms) {
  const t = document.getElementById("toast");
  t.textContent = msg;
  t.className = kind || "";
  t.hidden = false;
  clearTimeout(toast._timer);
  toast._timer = setTimeout(() => { t.hidden = true; }, ms || 4000);
}

// 路径 = 从 state.doc 出发的键数组（数组下标用字符串数字）
function getPath(obj, path) {
  let c = obj;
  for (const k of path) {
    if (c == null) return undefined;
    c = c[k];
  }
  return c;
}

function setPathRoot(path, val) {
  let c = state.doc;
  for (let i = 0; i < path.length - 1; i++) c = c[path[i]];
  c[path[path.length - 1]] = val;
  markDirty();
}

function delPathRoot(path) {
  let c = state.doc;
  for (let i = 0; i < path.length - 1; i++) c = c[path[i]];
  delete c[path[path.length - 1]];
  markDirty();
}

// 保序重命名对象键（表/字段/枚举/对象属性共用）
function renameKey(map, oldKey, newKey) {
  if (!map || !(oldKey in map) || newKey === oldKey || newKey in map || !newKey) return false;
  const entries = Object.entries(map);
  const i = entries.findIndex(([k]) => k === oldKey);
  const [entry] = entries.splice(i, 1);
  entries.splice(i, 0, [newKey, entry[1]]);
  const next = {};
  for (const [k, v] of entries) next[k] = v;
  return next;
}

function nextName(map, base) {
  if (!(base in map)) return base;
  for (let i = 2; i < 1000; i++) {
    const cand = `${base}_${i}`;
    if (!(cand in map)) return cand;
  }
  return `${base}_${Date.now()}`;
}

// ── 控件面板（逐个写回，外科手术式） ────────────────────────────────

// 文本输入（字符串）
function textInput(path, { toNull = false, dataPath } = {}) {
  const input = document.createElement("input");
  input.type = "text";
  const cur = getPath(state.doc, path);
  input.value = cur == null ? "" : String(cur);
  if (dataPath) input.setAttribute("data-path", dataPath);
  input.addEventListener("change", () => {
    const v = input.value.trim();
    if (v === "" && toNull) setPathRoot(path, null);
    else setPathRoot(path, v);
    if (toNull && v === "") input.value = "";
  });
  return input;
}

// 数字输入（min/max 等；空 → 删除键）
function numberInput(path, dataPath) {
  const input = document.createElement("input");
  input.type = "number";
  input.step = "any";
  const cur = getPath(state.doc, path);
  input.value = cur == null ? "" : String(cur);
  if (dataPath) input.setAttribute("data-path", dataPath);
  input.addEventListener("change", () => {
    const raw = input.value.trim();
    if (raw === "") { delPathRoot(path); return; }
    const n = Number(raw);
    if (Number.isFinite(n)) { setPathRoot(path, n); input.classList.remove("num-bad"); }
    else input.classList.add("num-bad");
  });
  return input;
}

// 整数输入（length/count 类）
function intInput(path, dataPath) {
  const input = numberInput(path, dataPath);
  input.step = "1";
  input.addEventListener("change", () => {
    const raw = input.value.trim();
    if (raw === "") return;
    const n = Number(raw);
    if (Number.isInteger(n)) { setPathRoot(path, n); input.classList.remove("num-bad"); }
    else input.classList.add("num-bad");
  });
  return input;
}

// 复选框（bool；反选时删除键 → canonical skip-when-false）
function boolInput(path, dataPath) {
  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = !!getPath(state.doc, path);
  if (dataPath) input.setAttribute("data-path", dataPath);
  input.addEventListener("change", () => {
    if (input.checked) setPathRoot(path, true);
    else delPathRoot(path);
  });
  return input;
}

// 逗号列表（primary_key/order_by/targets/fields 等 → 字符串数组）
function listInput(path, dataPath) {
  const input = document.createElement("input");
  input.type = "text";
  const cur = getPath(state.doc, path);
  input.value = Array.isArray(cur) ? cur.join(", ") : "";
  if (dataPath) input.setAttribute("data-path", dataPath);
  input.addEventListener("change", () => {
    const arr = input.value.split(",").map((s) => s.trim()).filter(Boolean);
    setPathRoot(path, arr);
  });
  return input;
}

// JSON 字面量（default / enum value 等：null | 字面量）
function jsonInput(path, dataPath) {
  const input = document.createElement("input");
  input.type = "text";
  input.placeholder = "null、数字、字符串、数组或对象字面量";
  const cur = getPath(state.doc, path);
  input.value = cur == null ? "null" : JSON.stringify(cur);
  if (dataPath) input.setAttribute("data-path", dataPath);
  input.addEventListener("change", () => {
    const raw = input.value.trim();
    if (raw === "") { setPathRoot(path, null); input.value = "null"; return; }
    try {
      setPathRoot(path, JSON.parse(raw));
      input.classList.remove("json-bad");
    } catch {
      input.classList.add("json-bad");
      toast("JSON 字面量无法解析，未写入", "err");
    }
  });
  return input;
}

// ── 类型编辑器（递归：Array / Object / Map / Enum） ─────────────────

function typeLabel(node) {
  if (!node || typeof node.kind !== "string") return "?";
  switch (node.kind) {
    case "Array": return `Array<${typeLabel(node.value)}>`;
    case "Object": return `Object{${Object.keys(node.value || {}).join(",")}}`;
    case "Map": {
      const v = node.value || {};
      return `Map<${v.key_type || "?"},${typeLabel(v.value_type)}>`;
    }
    case "Enum": return `Enum<${node.value ?? "?"}>`;
    default: return node.kind;
  }
}

function isTypeNode(v) {
  return !!v && typeof v === "object" && !Array.isArray(v)
    && typeof v.kind === "string" && KINDS.includes(v.kind);
}

function newTypePayload(kind, old) {
  switch (kind) {
    case "Array": {
      const v = old && isTypeNode(old.value) ? old.value : { kind: "String" };
      return { kind, value: v };
    }
    case "Object": {
      const v = old && old.value && typeof old.value === "object" && !Array.isArray(old.value) ? old.value : {};
      return { kind, value: v };
    }
    case "Map": {
      const v = old && old.value && old.value.key_type ? old.value : { key_type: "string", value_type: { kind: "String" } };
      return { kind, value: v };
    }
    case "Enum": return { kind, value: enumNames()[0] || "" };
    default: return { kind };
  }
}

// 渲染 type 节点编辑器；opts.path = type 节点在 doc 中的路径
function TypeEditor(opts) {
  const root = el("div", "type-editor");
  const kindRow = el("div", "type-kind-row");
  const kindSel = el("select");
  KINDS.forEach((k) => kindSel.append(option(k)));
  kindSel.setAttribute("data-path", opts.path.concat("kind").join("."));
  kindRow.append(kindSel);
  const payloadBox = el("div");
  root.append(kindRow, payloadBox);

  const nodePath = opts.path;
  const node = () => getPath(state.doc, nodePath);
  const setNode = (n) => setPathRoot(nodePath, n);

  function refresh() {
    const n = node() || { kind: "String" };
    kindSel.value = n.kind;
    payloadBox.replaceChildren();
    if (opts.depth >= MAX_DEPTH) {
      payloadBox.append(el("div", "type-kind-hint", "嵌套过深，此处不再展开（仍会原样保留）"));
      return;
    }
    switch (n.kind) {
      case "Array":
        payloadBox.append(typeSubEditor("元素类型", [...nodePath, "value"], opts.depth + 1));
        break;
      case "Object":
        payloadBox.append(objectPropsEditor(n, nodePath, opts.depth + 1));
        break;
      case "Map":
        payloadBox.append(mapEditor(n, nodePath, opts.depth + 1));
        break;
      case "Enum":
        payloadBox.append(enumRefEditor(n, nodePath));
        break;
    }
  }

  kindSel.addEventListener("change", () => {
    setNode(newTypePayload(kindSel.value, node(), opts));
    refresh();
  });

  refresh();
  return root;
}

function typeSubEditor(label, path, depth) {
  const wrap = el("div");
  wrap.append(el("div", "type-kind-hint", label));
  wrap.append(new TypeEditor({ path, depth }));
  return wrap;
}

function objectPropsEditor(node, nodePath, depth) {
  const box = el("div");
  const mapPath = [...nodePath, "value"];
  const props = getPath(state.doc, mapPath) || {};
  const render = () => {
    box.replaceChildren();
    for (const name of Object.keys(props)) {
      const row = el("div", "obj-prop-wrap");
      const head = el("div", "type-obj-prop");
      const nameInput = el("input");
      nameInput.type = "text";
      nameInput.value = name;
      nameInput.addEventListener("change", () => {
        const target = nameInput.value.trim();
        if (!target || target === name || target in props) { nameInput.value = name; return; }
        const next = renameKey(props, name, target);
        if (next) {
          setPathRoot(mapPath, next);
          refreshEntity();
        } else nameInput.value = name;
      });
      const delBtn = el("button", "icon danger", "×");
      delBtn.title = "删除属性";
      delBtn.addEventListener("click", () => {
        delPathRoot([...mapPath, name]);
        refreshEntity();
      });
      head.append(nameInput, delBtn);
      row.append(head);
      row.append(new TypeEditor({ path: [...mapPath, name], depth }));
      box.append(row);
    }
  };
  const addBtn = el("button");
  addBtn.textContent = "＋ 属性";
  addBtn.addEventListener("click", () => {
    const k = nextName(props, "prop");
    setPathRoot([...mapPath, k], { kind: "Any" });
    refreshEntity();
  });
  render();
  box.append(addBtn);
  return box;
}

function mapEditor(node, nodePath, depth) {
  const box = el("div");
  const keyRow = el("div", "row");
  keyRow.append(el("label", null, "键类型"));
  const keySel = el("select");
  keySel.append(option("string", "string（键为字符串）"), option("int", "int（键为整数）"));
  keySel.value = (node.value || {}).key_type || "string";
  keySel.setAttribute("data-path", nodePath.concat("value", "key_type").join("."));
  keySel.addEventListener("change", () => {
    if (!getPath(state.doc, [...nodePath, "value"])) setPathRoot(nodePath, newTypePayload("Map", node()));
    setPathRoot([...nodePath, "value", "key_type"], keySel.value);
  });
  keyRow.append(keySel);
  box.append(keyRow);
  box.append(typeSubEditor("值类型", [...nodePath, "value", "value_type"], depth));
  return box;
}

function enumRefEditor(node, nodePath) {
  const box = el("div", "row");
  box.append(el("label", null, "枚举"));
  const input = el("input");
  input.type = "text";
  input.setAttribute("list", "enum-names");
  input.value = node.value ?? "";
  input.setAttribute("data-path", nodePath.concat("value").join("."));
  input.placeholder = "枚举名（可输入新名）";
  input.addEventListener("change", () => setPathRoot([...nodePath, "value"], input.value));
  box.append(input);
  return box;
}

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
  fsec.append(ftable);
  // 字段编辑器容器（选中字段时填充）
  const fed = el("div");
  fed.id = "field-editor";
  fsec.append(addField, fed);
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

// ── 校验与保存 ──────────────────────────────────────────────────────

async function validate() {
  const docText = JSON.stringify(state.doc);
  const { status, data } = await api("/api/validate", docText);
  void status;
  state.diags = (data && data.diagnostics) || [];
  renderDiags(false);
  const errs = state.diags.filter((d) => d.severity === "error").length;
  const warns = state.diags.filter((d) => d.severity === "warning").length;
  if (data && data.ok) toast("校验通过（L1 无错误）", "ok");
  else toast(`校验未通过：${errs} 错误 · ${warns} 警告`, "err");
}

async function save() {
  const docText = JSON.stringify(state.doc);
  const { status, data } = await api("/api/schema", docText);
  if (data && data.ok) {
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