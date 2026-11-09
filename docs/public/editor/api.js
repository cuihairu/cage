// HTTP API 封装、脏标记与自动重校验调度（split out of app.js, LAY-027）。

import { state } from "./state.js";
import { updateScopeNote, validate } from "./app.js";

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
  // 作用域投影按「已保存的 schema」计算，未保存改动让它显式说明。
  if (state.scope) updateScopeNote();
  scheduleAutoValidate();
}

// 改动即时重校验（策划界面 P1）：700ms 防抖，面板静默更新不打扰；
// 开关默认开，手动「校验」永远可用。
let autoValidateTimer = 0;
// 清掉未决的自动重校验（ESM 导入绑定只读，归零只能在这里做）。
function resetAutoValidate() {
  clearTimeout(autoValidateTimer);
  autoValidateTimer = 0;
}
function scheduleAutoValidate() {
  const box = document.getElementById("auto-validate");
  if (!box || !box.checked) return;
  clearTimeout(autoValidateTimer);
  autoValidateTimer = setTimeout(() => { autoValidateTimer = 0; validate(true); }, 700);
}


export { api, markDirty, resetAutoValidate, scheduleAutoValidate };
