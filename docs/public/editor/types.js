// 类型编辑器（递归：Array / Object / Map / Enum）（split out of app.js, LAY-027）。

import { KINDS, MAX_DEPTH } from "./constants.js";
import { enumNames, state } from "./state.js";
import { delPathRoot, el, getPath, nextName, option, renameKey, setPathRoot } from "./dom.js";
import { refreshEntity } from "./forms.js";

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

export { typeLabel, isTypeNode, newTypePayload, TypeEditor, typeSubEditor, objectPropsEditor, mapEditor, enumRefEditor };
