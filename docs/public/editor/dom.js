// 基础 DOM 工具、路径读写与控件输入面板（split out of app.js, LAY-027）。

import { state } from "./state.js";
import { markDirty } from "./api.js";

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

// targets 徽标（U2b）：field/table 的 profile 可见性列表原样成 chip，
// 数据源就是 schema 本身（targets: ["server"] → server only），无新状态。
function targetsChips(targets) {
  if (!targets || !targets.length) return null;
  const wrap = el("span", "targets-chips");
  for (const t of targets) wrap.append(el("span", "targets-chip", t));
  return wrap;
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

export { el, option, targetsChips, toast, getPath, setPathRoot, delPathRoot, renameKey, nextName, textInput, numberInput, intInput, boolInput, listInput, jsonInput };
