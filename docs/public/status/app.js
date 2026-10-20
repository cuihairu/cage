// cage 项目状态页 —— vanilla ES module，零依赖，与 Schema 编辑器同一风格。
//
// 数据面 = GET /api/status（校验面板 + 每 profile 构建事实）与
// POST /api/build（触发构建，与 `cage build` 同一 build_project 入口）。
// 校验走 `cage check` 的单一入口 validate_project——页面、CLI、LSP
// 三处消费同一份诊断，永不分歧。前端不复制任何规则语义。

"use strict";

const state = {
  meta: null,       // 最近一次 GET /api/status 负荷
  building: null,   // 正在构建的 profile 名（防重入）
  prevRun: null,    // 上一次校验运行的诊断键（诊断对比 P2 用）
  tables: null,     // 最近一次 GET /api/tables 负荷（数据面板）
  table: null,      // 当前展示的表：{ name, offset, limit, payload }
};

// ── 基础工具 ────────────────────────────────────────────────────────

function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
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

function toast(msg, kind, ms) {
  const t = document.getElementById("toast");
  t.textContent = msg;
  t.className = kind || "";
  t.hidden = false;
  clearTimeout(toast._timer);
  toast._timer = setTimeout(() => { t.hidden = true; }, ms || 4000);
}

function shortHash(s) {
  return typeof s === "string" && s.length > 12 ? s.slice(0, 12) : (s || "–");
}

// 诊断定位的扁平文本：location（file/sheet/row/col/field）优先，
// 顶层 table/row/column 兜底。
function locText(d) {
  const parts = [];
  const loc = d.location;
  if (loc) {
    if (loc.file) parts.push(loc.file);
    if (loc.sheet) parts.push(`sheet ${loc.sheet}`);
    if (loc.row != null && loc.row !== "") parts.push(`行 ${loc.row}`);
    if (loc.col != null && loc.col !== "") parts.push(`列 ${loc.col}`);
    if (loc.field) parts.push(loc.field);
  }
  if (d.table && !parts.some((p) => p === d.table)) parts.push(`表 ${d.table}`);
  if (d.row && !loc) parts.push(`行 ${d.row}`);
  if (d.column) parts.push(d.column);
  if (d.field && !loc) parts.push(d.field);
  return parts.join(" · ");
}

// ── 渲染：校验面板 ──────────────────────────────────────────────────

// 诊断对比（策划界面 P2）：会话内两次校验的新增/消失。同一
// profile+env+level 的两次运行才比对；跨快照的历史诊断不存储（那是
// 新服务面，无用户令），产物对比走既有 /api/diff。构建失败路径换掉
// validation 时同样走这里，正好覆盖「改数据 → 构建 → 看新增错误」。
function diagKey(d) {
  return [d.code, d.table, d.row, d.column, d.field, d.message].join("|");
}

function markDiagDiff(v) {
  if (!v) return;
  const runKey = [
    v.profile || (state.meta && state.meta.profile) || "",
    document.getElementById("env-in").value.trim(),
    document.getElementById("level-sel").value,
  ].join("|");
  const keys = (v.diagnostics || []).map(diagKey);
  const prev = state.prevRun;
  if (prev && prev.runKey === runKey && v.diagnostics) {
    const prevSet = new Set(prev.keys);
    const curSet = new Set(keys);
    const added = keys.filter((k) => !prevSet.has(k));
    const gone = prev.keys.filter((k) => !curSet.has(k));
    v.diff = { addedSet: new Set(added), addedCount: added.length, goneCount: gone.length };
  }
  state.prevRun = { runKey, keys };
  return v;
}

function renderValidation() {
  const summary = document.getElementById("val-summary");
  const list = document.getElementById("val-diags");
  const v = state.meta && state.meta.validation;
  list.replaceChildren();

  if (!v) {
    summary.textContent = "尚未加载";
    return;
  }
  if (v.error) {
    summary.textContent = "项目装载失败";
    const li = el("li", "diag-severity-error");
    li.append(el("div", "diag-msg", v.error));
    list.append(li);
    return;
  }

  const bits = [];
  if (v.errors) bits.push(`${v.errors} 错误`);
  if (v.warnings) bits.push(`${v.warnings} 警告`);
  if (v.info) bits.push(`${v.info} 信息`);
  bits.push(`${v.visible_tables} 张表可见`);
  summary.replaceChildren();
  summary.className = v.ok ? "badge ok" : "badge err";
  summary.textContent = v.ok ? `通过 · ${bits.join(" · ")}` : bits.join(" · ");
  if (v.diff) {
    const parts = [];
    if (v.diff.addedCount) parts.push(`新增 ${v.diff.addedCount}`);
    if (v.diff.goneCount) parts.push(`消失 ${v.diff.goneCount}`);
    if (parts.length) summary.append(el("span", "diag-new", `较上次 ${parts.join(" / ")}`));
  }

  if (!v.diagnostics || v.diagnostics.length === 0) {
    const li = el("li", "", "无诊断");
    li.style.cursor = "default";
    li.style.color = "var(--text-dim)";
    list.append(li);
    return;
  }
  // 过滤（数据面板 P0）：严重级下拉 + 错误码子串，纯前端过滤——
  // 诊断数据永远整份来自 /api/status，过滤不触发新请求。
  const sev = document.getElementById("diag-sev").value;
  const code = document.getElementById("diag-code").value.trim().toUpperCase();
  const shown = v.diagnostics.filter((d) =>
    (!sev || (d.severity || "info") === sev)
    && (!code || (d.code || "").toUpperCase().includes(code)));
  if (shown.length !== v.diagnostics.length) {
    summary.append(el("span", "muted", `　显示 ${shown.length}/${v.diagnostics.length}`));
  }
  if (shown.length === 0) {
    const li = el("li", "", "当前过滤条件下无诊断");
    li.style.cursor = "default";
    li.style.color = "var(--text-dim)";
    list.append(li);
    return;
  }
  shown.forEach((d) => {
    const li = el("li", `diag-severity-${d.severity || "info"} diag-click`);
    const line1 = el("div", "diag-line1");
    line1.append(el("span", "diag-code", d.code || "?"), el("div", "diag-msg", d.message || ""));
    if (v.diff && v.diff.addedSet && v.diff.addedSet.has(diagKey(d))) {
      line1.append(el("span", "diag-new", "新"));
    }
    li.append(line1);
    const loc = locText(d);
    if (loc) li.append(el("div", "diag-loc", loc));
    if (d.hint) li.append(el("div", "diag-hint", `hint: ${d.hint}`));
    if (d.table) {
      li.title = "点击在数据面板定位该诊断的表";
      li.addEventListener("click", () => jumpToDiag(d));
    }
    list.append(li);
  });
}

// ── 渲染：构建卡片 ──────────────────────────────────────────────────

function renderBuilds() {
  const wrap = document.getElementById("build-cards");
  const summary = document.getElementById("build-summary");
  const builds = (state.meta && state.meta.builds) || {};
  wrap.replaceChildren();

  const names = Object.keys(builds);
  const freshCount = names.filter((n) => builds[n] && builds[n].fresh).length;
  summary.textContent = names.length
    ? `${freshCount}/${names.length} 个 profile 与上次构建一致`
    : "–";

  names.forEach((name) => {
    const b = builds[name];
    if (!b || b.ok === false) {
      const card = el("div", "card");
      card.append(el("h3", "", name), el("div", "card-note", (b && b.error) || "状态不可用"));
      wrap.append(card);
      return;
    }
    const card = el("div", "card");

    const head = el("div", "card-head");
    head.append(el("h3", "", name));
    const badge = el("span", "badge");
    if (b.fresh) {
      badge.className = "badge ok";
      badge.textContent = "与上次构建一致";
    } else if (b.last_build_profile === name) {
      badge.className = "badge warn";
      badge.textContent = "已过期";
    } else {
      badge.textContent = "未构建";
    }
    head.append(badge);
    if (b.last_build_profile && b.last_build_profile !== name) {
      head.append(el("span", "muted", `（manifest 当前属于 ${b.last_build_profile}）`));
    }
    const spacer = el("span", "spacer");
    head.append(spacer);
    const buildBtn = el("button", "primary", state.building === name ? "构建中…" : "构建");
    buildBtn.disabled = state.building !== null;
    buildBtn.addEventListener("click", () => buildProfile(name));
    head.append(buildBtn);
    card.append(head);

    const chips = el("div", "chips");
    (b.targets || []).forEach((t) => {
      chips.append(el("span", "chip", `${t.format} → ${t.output_dir}`));
    });
    card.append(chips);

    if (b.changed_tables && b.changed_tables.length > 0) {
      const row = el("div", "chips");
      row.append(el("span", "muted", "已变化的表："));
      b.changed_tables.forEach((t) => row.append(el("span", "chip stale", t)));
      card.append(row);
    }

    const facts = el("div", "facts");
    const fact = (label, value) => {
      facts.append(el("span", "", label), el("b", "", value));
    };
    if (b.build_id) {
      fact("build_id", shortHash(b.build_id));
      fact("content_hash", shortHash(b.content_hash));
      fact("产物", `${b.artifact_count} 个文件 · ${b.target_count} 个 target`);
    } else {
      fact("manifest", "尚无构建记录");
    }
    card.append(facts);
    wrap.append(card);
  });
}

// ── 数据面板（策划界面 P0）─────────────────────────────────────────
// 表清单与行数据来自 GET /api/tables 与 GET /api/table（同一装载链，
// 可见性与 CLI 完全一致）；行分页在服务端截断，前端只渲染返回页。

function panelQuery(extra) {
  const params = new URLSearchParams();
  const profile = document.getElementById("profile-sel").value;
  const env = document.getElementById("env-in").value.trim();
  if (profile) params.set("profile", profile);
  if (env) params.set("env", env);
  for (const [k, v] of Object.entries(extra || {})) params.set(k, v);
  const qs = params.toString();
  return qs ? `?${qs}` : "";
}

async function loadTables() {
  const { data } = await api(`/api/tables${panelQuery()}`);
  state.tables = data && data.ok ? data : null;
  renderTableList();
}

function renderTableList() {
  const list = document.getElementById("table-list");
  const summary = document.getElementById("data-summary");
  list.replaceChildren();
  if (!state.tables) {
    summary.textContent = "表清单不可用";
    return;
  }
  const tables = state.tables.tables || [];
  summary.textContent = `${tables.length} 张表 · profile ${state.tables.profile}`;
  if (!tables.length) {
    const li = el("li", "", "（该 profile 下无可见表）");
    li.style.color = "var(--text-dim)";
    list.append(li);
    return;
  }
  tables.forEach((t) => {
    const li = el("li", "table-item" + (state.table && state.table.name === t.name ? " sel" : ""));
    li.append(
      el("div", "table-name", t.name),
      el("div", "table-facts", `${t.rows} 行 · ${t.source_file || "?"}${t.sheet ? ` · sheet ${t.sheet}` : ""}`)
    );
    li.addEventListener("click", () => showTable(t.name, 0));
    list.append(li);
  });
}

async function showTable(name, offset) {
  const limit = 200;
  const { data } = await api(`/api/table${panelQuery({ name, offset, limit })}`);
  const meta = document.getElementById("table-meta");
  if (!data || !data.ok) {
    meta.textContent = `无法读取表 ${name}：${(data && data.error) || "服务错误"}`;
    document.getElementById("table-grid").replaceChildren();
    document.getElementById("table-pager").hidden = true;
    return;
  }
  state.table = { name, offset: data.offset, limit: data.limit, payload: data };
  meta.replaceChildren();
  meta.append(
    el("strong", "", name),
    el("span", "", `　${data.total} 行 · 主键 ${(data.primary_key || []).join(", ") || "–"} · ${data.source_file || "?"}${data.sheet ? ` · sheet ${data.sheet}` : ""}`)
  );
  renderGrid(data);
  renderPager(data);
  renderTableList();
}

function cellText(v) {
  if (v === null || v === undefined) return "";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

function renderGrid(data) {
  const table = document.getElementById("table-grid");
  table.replaceChildren();
  const cols = data.columns || [];
  const thead = el("thead");
  const hr = el("tr");
  hr.append(el("th", "", "#"));
  cols.forEach((c) => hr.append(el("th", "", c)));
  thead.append(hr);
  table.append(thead);
  const tbody = el("tbody");
  (data.rows || []).forEach((r) => {
    const tr = el("tr");
    tr.dataset.index = r.index;
    tr.append(el("td", "row-idx", String(r.index)));
    cols.forEach((c) => {
      const v = (r.cells || {})[c];
      tr.append(el("td", "cell-num-null-" + (v === null ? "1" : "0"), cellText(v)));
    });
    tbody.append(tr);
  });
  table.append(tbody);
}

function renderPager(data) {
  const pager = document.getElementById("table-pager");
  const { offset, limit, total } = data;
  if (total <= limit) { pager.hidden = true; return; }
  pager.hidden = false;
  const from = total === 0 ? 0 : offset + 1;
  const to = Math.min(offset + limit, total);
  document.getElementById("pg-info").textContent = `第 ${from}–${to} / ${total} 行`;
  const prev = document.getElementById("pg-prev");
  const next = document.getElementById("pg-next");
  prev.disabled = offset === 0;
  next.disabled = to >= total;
}

// 诊断点位：跳到诊断所指的表并高亮目标行。d.row 是「主键值或行号」
// （字符串形态），先取一页大窗口定位，再按定位结果落到常规分页。
async function jumpToDiag(d) {
  const want = String(d.row == null ? "" : d.row);
  const { data } = await api(`/api/table${panelQuery({ name: d.table, offset: 0, limit: 1000 })}`);
  if (!data || !data.ok) {
    toast(`无法定位表 ${d.table}：${(data && data.error) || "服务错误"}`, "err");
    return;
  }
  const rows = data.rows || [];
  const pkCols = data.primary_key || [];
  const hit = rows.find((r) =>
    (want !== "" && (r.pk || []).some((v) => cellText(v) === want))
    || (want !== "" && String(r.index) === want)
    || (want === "" && pkCols.length === 0 && r.index === 0));
  const limit = 200;
  const targetIndex = hit ? hit.index : 0;
  const offset = Math.floor(targetIndex / limit) * limit;
  if (offset === 0 && data.total <= limit) {
    state.table = { name: d.table, offset: 0, limit, payload: data };
  } else {
    await showTable(d.table, offset);
  }
  document.getElementById("data").scrollIntoView({ behavior: "smooth", block: "start" });
  requestAnimationFrame(() => {
    const tr = document.querySelector(`#table-grid tbody tr[data-index="${targetIndex}"]`);
    if (!tr) return;
    tr.classList.remove("hl");
    void tr.offsetWidth;
    tr.classList.add("hl");
    tr.scrollIntoView({ behavior: "smooth", block: "center" });
  });
}

// ── 动作 ────────────────────────────────────────────────────────────

async function refresh() {
  const params = new URLSearchParams();
  const profile = document.getElementById("profile-sel").value;
  const level = document.getElementById("level-sel").value;
  const env = document.getElementById("env-in").value.trim();
  if (profile) params.set("profile", profile);
  if (level) params.set("level", level);
  if (env) params.set("env", env);
  const qs = params.toString();
  const { data } = await api(`/api/status${qs ? `?${qs}` : ""}`);
  if (!data || !data.ok) {
    const fatal = document.getElementById("fatal-error");
    fatal.hidden = false;
    fatal.textContent = `无法读取状态：${(data && data.error) || "服务错误"}`;
    return;
  }
  document.getElementById("fatal-error").hidden = true;
  const first = !state.meta;
  state.meta = data;
  markDiagDiff(state.meta.validation);
  document.getElementById("meta-project").textContent = data.project || "?";
  document.getElementById("meta-path").textContent =
    `schema: ${data.schema_path || "未配置"}`;
  document.getElementById("meta-output").textContent = `output_dir: ${data.output_dir || "build"}`;

  // profile 下拉只装一次（配置快照在服务端每请求重载，名字集合稳定）
  const sel = document.getElementById("profile-sel");
  if (first) {
    (data.profile_names || []).forEach((p) => {
      const o = el("option", "", p);
      o.value = p;
      sel.append(o);
    });
    sel.value = data.profile;
  }
  renderValidation();
  renderBuilds();
  await loadTables();
  await loadArtifacts();
  await loadSnapshots();
  await loadRegistry();
}

async function buildProfile(name) {
  if (state.building) return;
  state.building = name;
  renderBuilds();
  const env = document.getElementById("env-in").value.trim();
  const body = JSON.stringify({ profile: name, env: env || null, incremental: true });
  const { status, data } = await api("/api/build", body);
  void status;
  state.building = null;

  if (data && data.ok && data.up_to_date) {
    toast(`构建：${name} 已是最新（${data.artifact_count} 个产物），未重新生成`, "ok", 6000);
  } else if (data && data.ok) {
    const parts = [`${data.regenerated} 重新生成`];
    if (data.carried) parts.push(`${data.carried} 沿用`);
    toast(`构建完成：${name} · ${data.artifact_count} 个产物（${parts.join("，")}）`, "ok", 6000);
  } else if (data && data.diagnostics) {
    // 校验失败：诊断直接进面板（与 /api/validate 同一形状）
    if (state.meta) {
      state.meta.validation = markDiagDiff({
        ok: false,
        profile: name,
        errors: data.diagnostics.filter((d) => d.severity === "error").length,
        warnings: data.diagnostics.filter((d) => d.severity === "warning").length,
        info: data.diagnostics.filter((d) => d.severity === "info").length,
        visible_tables: (state.meta.validation && state.meta.validation.visible_tables) || 0,
        diagnostics: data.diagnostics,
      });
    }
    renderValidation();
    toast(`构建被拒：${name} 未通过校验（见校验面板）`, "err", 7000);
  } else {
    toast(`构建失败：${(data && data.error) || "服务错误"}`, "err", 7000);
  }
  await refresh();
}

// ── 渲染：产物区（U2a）─────────────────────────────────────────────

function humanSize(n) {
  if (n == null) return "–";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(2)} MB`;
}

function artifactMimeLabel(path) {
  if (path.endsWith(".msgpack")) return "二进制";
  if (path.endsWith(".json")) return "JSON";
  if (path.endsWith(".csv")) return "CSV";
  return "文本";
}

async function showPreview(rel, btn) {
  const pre = document.getElementById("artifact-preview");
  let res;
  try {
    res = await fetch(`/api/artifact?path=${encodeURIComponent(rel)}`);
  } catch (e) {
    pre.hidden = false;
    pre.textContent = `无法连接：${e}`;
    return;
  }
  const raw = await res.arrayBuffer();
  if (!res.ok) {
    let msg = `HTTP ${res.status}`;
    try { msg = JSON.parse(new TextDecoder().decode(raw)).error || msg; } catch {}
    pre.hidden = false;
    pre.textContent = `无法读取产物：${msg}`;
    return;
  }
  pre.hidden = false;
  document.getElementById("artifact-preview-head").textContent = rel;
  if (rel.endsWith(".msgpack")) {
    pre.textContent = `二进制产物（${humanSize(raw.byteLength)}），不预渲染——用「下载」取文件。`;
  } else {
    const text = new TextDecoder().decode(raw);
    if (rel.endsWith(".json")) {
      try { pre.textContent = JSON.stringify(JSON.parse(text), null, 2); }
      catch { pre.textContent = text; }
    } else {
      pre.textContent = text;
    }
  }
  btn.scrollIntoView({ behavior: "smooth", block: "nearest" });
}

async function loadArtifacts() {
  const summary = document.getElementById("artifact-summary");
  const note = document.getElementById("artifact-note");
  const groups = document.getElementById("artifact-groups");
  const preview = document.getElementById("artifact-preview");
  const profile = document.getElementById("profile-sel").value;
  const env = document.getElementById("env-in").value.trim();
  summary.textContent = "–";
  note.hidden = true;
  groups.replaceChildren();
  preview.hidden = true;
  if (!profile) return;

  const qs = new URLSearchParams({ profile });
  if (env) qs.set("env", env);
  const { data } = await api(`/api/artifacts?${qs.toString()}`);
  if (!data || !data.ok) {
    summary.textContent = "不可用";
    return;
  }

  if (data.manifest_profile !== profile) {
    summary.textContent = "未构建";
    note.hidden = false;
    note.textContent = data.manifest_profile
      ? `manifest 当前属于 ${data.manifest_profile}——构建 ${profile} 后即可浏览其产物。`
      : "尚无构建记录——构建后即可浏览产物。";
    return;
  }

  const arts = data.artifacts || [];
  summary.textContent = `${arts.length} 个文件 · build_id ${shortHash(data.build_id)}`;
  if (data.stale) {
    note.hidden = false;
    note.textContent = "产物已过期：源数据在上次构建后有变化（见构建状态）——浏览前建议重新构建。";
  }

  // 按 target output_dir 分组（manifest target 顺序 = profile 配置顺序）
  const byDir = new Map();
  for (const t of data.targets || []) byDir.set(t.output_dir, { t, files: [] });
  const rest = [];
  for (const a of arts) {
    const hit = [...byDir.keys()].find((d) => a.path === d || a.path.startsWith(d + "/"));
    if (hit) byDir.get(hit).files.push(a);
    else rest.push(a);
  }

  for (const { t, files } of byDir.values()) {
    const card = el("div", "card");
    const head = el("div", "card-head");
    head.append(el("h3", "", `${t.format} → ${t.output_dir}`));
    head.append(el("span", "muted", `${files.length} 个文件`));
    if (files.length > 1) {
      // 一键导出（策划界面 P1）：逐个触发 /api/artifact 下载（服务端
      // 每个文件先验 blake3 再发）；不新增打包端点/依赖，浏览器可能
      // 请求「允许下载多个文件」。
      const all = el("button", "mini", "一键导出");
      all.title = "下载该 target 的全部产物（逐个文件，各自先验哈希）";
      all.addEventListener("click", () => {
        files.forEach((a, i) => {
          setTimeout(() => {
            const dl = document.createElement("a");
            dl.href = `/api/artifact?path=${encodeURIComponent(a.path)}`;
            dl.download = a.path.split("/").pop();
            document.body.append(dl);
            dl.click();
            dl.remove();
          }, i * 350);
        });
      });
      head.append(all);
    }
    card.append(head);
    const chips = el("div", "chips");
    files.forEach((a) => {
      const chip = el("span", "chip artifact");
      chip.title = `blake3 ${a.hash}`;
      const name = el("span", "artifact-name", a.path);
      const size = el("span", "artifact-size", humanSize(a.size));
      const btn = el("button", "mini", "预览");
      btn.addEventListener("click", () => showPreview(a.path, btn));
      const dl = el("a", "", "下载");
      dl.href = `/api/artifact?path=${encodeURIComponent(a.path)}`;
      dl.download = a.path.split("/").pop();
      chip.append(name, size, btn, dl);
      chips.append(chip);
    });
    if (files.length === 0) chips.append(el("span", "muted", "（无文件）"));
    card.append(chips);
    groups.append(card);
  }
}

// ── 快照（U3 列 / 验 / 打包）────────────────────────────────────────

let snapshotsCache = []; // 最近一次 GET /api/snapshots 负荷（对比下拉也用它）

async function loadSnapshots() {
  const summary = document.getElementById("snap-summary");
  const list = document.getElementById("snap-list");
  const { data } = await api("/api/snapshots");
  list.replaceChildren();
  if (!data || !data.ok) {
    summary.textContent = "不可用";
    return;
  }
  snapshotsCache = data.snapshots || [];
  summary.textContent = snapshotsCache.length
    ? `${snapshotsCache.length} 个快照（${data.output_dir}/snapshot/）`
    : `尚无快照（${data.output_dir}/snapshot/）`;

  snapshotsCache.forEach((s) => {
    const card = el("div", "card");
    const head = el("div", "card-head");
    head.append(el("h3", "", s.name));
    const badge = el("span", "badge");
    if (s.verified) {
      badge.className = "badge ok";
      badge.textContent = `verified · ${s.files_checked} files`;
    } else {
      badge.className = "badge err";
      badge.textContent = `FAILED · ${(s.problems || []).length} problem(s)`;
    }
    head.append(badge);
    card.append(head);
    if (!s.verified && (s.problems || []).length) {
      const pre = el("div", "card-note");
      pre.textContent = (s.problems || []).join("；");
      card.append(pre);
    }
    const chips = el("div", "chips");
    if (s.profile) chips.append(el("span", "chip", `profile ${s.profile}${s.environment ? ` / env ${s.environment}` : ""}`));
    if (s.build_id) chips.append(el("span", "chip", `build ${shortHash(s.build_id)}`));
    if (s.content_hash) chips.append(el("span", "chip", `content ${shortHash(s.content_hash)}`));
    if (s.artifact_count != null) chips.append(el("span", "chip", `${s.artifact_count} 产物`));
    card.append(chips);
    list.append(card);
  });

  renderDiffOptions();
}

async function packSnapshot() {
  const profile = document.getElementById("profile-sel").value;
  const env = document.getElementById("env-in").value.trim();
  const body = JSON.stringify({ profile: profile || null, env: env || null });
  const btn = document.getElementById("pack-btn");
  btn.disabled = true;
  btn.textContent = "打包中…";
  const { data } = await api("/api/snapshot", body);
  btn.disabled = false;
  btn.textContent = "打包快照";
  if (data && data.ok) {
    toast(`快照已打包：${data.snapshot_dir}（${data.files} 个文件，自验通过）`, "ok", 7000);
  } else if (data && data.diagnostics) {
    if (state.meta) {
      state.meta.validation = {
        ok: false,
        profile,
        errors: data.diagnostics.filter((d) => d.severity === "error").length,
        warnings: data.diagnostics.filter((d) => d.severity === "warning").length,
        info: data.diagnostics.filter((d) => d.severity === "info").length,
        visible_tables: (state.meta.validation && state.meta.validation.visible_tables) || 0,
        diagnostics: data.diagnostics,
      };
    }
    renderValidation();
    toast("快照被拒：构建未通过校验（见校验面板）", "err", 7000);
  } else {
    toast(`打包失败：${(data && data.error) || "服务错误"}`, "err", 7000);
  }
  await loadSnapshots();
}

// ── 对比（U3 分层 diff：表 → target → 文件）────────────────────────

function renderDiffOptions() {
  const fromSel = document.getElementById("diff-from");
  const toSel = document.getElementById("diff-to");
  const keep = (sel) => sel.value;
  const prevFrom = keep(fromSel);
  const prevTo = keep(toSel);
  const fill = (sel) => {
    sel.replaceChildren();
    const live = el("option", "", `当前构建（${(state.meta && state.meta.output_dir) || "build"}）`);
    live.value = (state.meta && state.meta.output_dir) || "build";
    sel.append(live);
    snapshotsCache.forEach((s) => {
      const o = el("option", "", `快照 ${s.name}`);
      o.value = `${(state.meta && state.meta.output_dir) || "build"}/snapshot/${s.name}`;
      sel.append(o);
    });
  };
  fill(fromSel);
  fill(toSel);
  const restore = (sel, prev) => {
    if ([...sel.options].some((o) => o.value === prev)) sel.value = prev;
  };
  restore(fromSel, prevFrom);
  restore(toSel, prevTo);
  // 默认基线 = 当前构建，目标 = 最新快照（有快照时）
  if (!prevFrom && snapshotsCache.length) fromSel.selectedIndex = 0;
  if (!prevTo && snapshotsCache.length) toSel.selectedIndex = fromSel.selectedIndex ? 0 : Math.min(1, toSel.options.length - 1);
}

function diffFileRow(f) {
  const how = f.from_hash === undefined ? "added"
    : f.to_hash === undefined ? "removed"
    : f.from_hash === f.to_hash ? "same" : "changed";
  const mark = how === "added" ? "+" : how === "removed" ? "−" : "~";
  const row = el("div", "diff-file");
  row.append(el("span", `diff-mark diff-${how}`, mark), el("span", "diff-path", f.path));
  if (how === "changed") {
    row.append(el("span", "muted", `${shortHash(f.from_hash)} → ${shortHash(f.to_hash)}`));
  }
  return row;
}

async function runDiff() {
  const from = document.getElementById("diff-from").value;
  const to = document.getElementById("diff-to").value;
  const result = document.getElementById("diff-result");
  const summary = document.getElementById("diff-summary");
  if (!from || !to) {
    toast("选择基线与目标再对比", "err");
    return;
  }
  if (from === to) {
    toast("基线与目标是同一份 manifest", "err");
    return;
  }
  summary.textContent = "对比中…";
  const qs = new URLSearchParams({ from, to });
  const { data } = await api(`/api/diff?${qs.toString()}`);
  result.replaceChildren();
  if (!data || !data.ok) {
    summary.textContent = "不可用";
    const err = el("div", "card-note");
    err.textContent = (data && data.error) || "服务错误";
    result.append(err);
    return;
  }

  const files = [
    ...(data.files.added || []),
    ...(data.files.removed || []),
    ...(data.files.changed || []),
  ];
  const unchanged = data.roots && data.roots.build_id && data.roots.build_id.from === data.roots.build_id.to;
  summary.textContent = unchanged && files.length === 0
    ? "两次构建逐字节一致"
    : `${data.changed_tables.length} 张表 · ${data.affected_targets.length} 个 target · ${files.length} 个文件有变化`;

  // 根：build_id / content_hash 的 from → to
  const roots = el("div", "facts");
  const fact = (label, a, b) => {
    roots.append(el("span", "", label), el("b", "", a === b ? shortHash(a) : `${shortHash(a)} → ${shortHash(b)}`));
  };
  fact("build_id", data.roots.build_id.from, data.roots.build_id.to);
  fact("content_hash", data.roots.content_hash.from, data.roots.content_hash.to);
  result.append(roots);

  if (files.length === 0) return;

  const chips = el("div", "chips");
  (data.changed_tables || []).forEach((t) => chips.append(el("span", "chip stale", t)));
  (data.affected_targets || []).forEach((t) => chips.append(el("span", "chip", t)));
  result.append(chips);

  // 三层折叠：表 → target → 文件
  const byTable = new Map();
  for (const f of files) {
    const table = f.table || "（无表归属）";
    if (!byTable.has(table)) byTable.set(table, []);
    byTable.get(table).push(f);
  }
  for (const [table, list] of byTable) {
    const det = el("details", "diff-table");
    det.append(el("summary", "", `${table}（${list.length} 个文件）`));
    const byTarget = new Map();
    for (const f of list) {
      const target = f.target || "（未知 target）";
      if (!byTarget.has(target)) byTarget.set(target, []);
      byTarget.get(target).push(f);
    }
    for (const [target, tfiles] of byTarget) {
      const inner = el("details", "diff-target");
      inner.append(el("summary", "", `${target}（${tfiles.length}）`));
      tfiles.forEach((f) => inner.append(diffFileRow(f)));
      det.append(inner);
    }
    result.append(det);
  }
}

// ── Registry 浏览（U3）──────────────────────────────────────────────

async function loadRegistry() {
  const summary = document.getElementById("registry-summary");
  const list = document.getElementById("registry-list");
  const { data } = await api("/api/registry");
  list.replaceChildren();
  if (!data || !data.ok) {
    summary.textContent = "不可用";
    const note = el("div", "card-note");
    note.textContent = (data && data.error) || "服务错误";
    list.append(note);
    return;
  }
  const packages = data.packages || [];
  if (packages.length === 0) {
    summary.textContent = data.note || "空";
    return;
  }
  summary.textContent = `${packages.length} 个包（registry ${data.registry}）`;
  packages.forEach((p) => {
    const card = el("div", "card");
    const head = el("div", "card-head");
    head.append(el("h3", "", p.package));
    head.append(el("span", "muted", `${(p.entries || []).length} 个版本`));
    card.append(head);
    const chips = el("div", "chips");
    (p.entries || []).forEach((v) => {
      const chip = el("span", "chip", `v${v.version} · build ${shortHash(v.build_id)} · ${v.files} files`);
      chip.title = `content_hash ${v.content_hash}`;
      chips.append(chip);
    });
    card.append(chips);
    list.append(card);
  });
}

// ── 启动 ────────────────────────────────────────────────────────────

async function init() {
  document.getElementById("refresh-btn").addEventListener("click", refresh);
  document.getElementById("profile-sel").addEventListener("change", refresh);
  document.getElementById("level-sel").addEventListener("change", refresh);
  document.getElementById("env-in").addEventListener("change", refresh);
  document.getElementById("diag-sev").addEventListener("change", () => renderValidation());
  document.getElementById("diag-code").addEventListener("input", () => renderValidation());
  document.getElementById("pg-prev").addEventListener("click", () => {
    if (state.table) showTable(state.table.name, Math.max(0, state.table.offset - state.table.limit));
  });
  document.getElementById("pg-next").addEventListener("click", () => {
    if (state.table) showTable(state.table.name, state.table.offset + state.table.limit);
  });
  document.getElementById("pack-btn").addEventListener("click", packSnapshot);
  document.getElementById("diff-btn").addEventListener("click", runDiff);
  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      refresh();
    }
  });
  await refresh();
}

init();
