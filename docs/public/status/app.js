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

  if (!v.diagnostics || v.diagnostics.length === 0) {
    const li = el("li", "", "无诊断");
    li.style.cursor = "default";
    li.style.color = "var(--text-dim)";
    list.append(li);
    return;
  }
  v.diagnostics.forEach((d) => {
    const li = el("li", `diag-severity-${d.severity || "info"}`);
    const line1 = el("div", "diag-line1");
    line1.append(el("span", "diag-code", d.code || "?"), el("div", "diag-msg", d.message || ""));
    li.append(line1);
    const loc = locText(d);
    if (loc) li.append(el("div", "diag-loc", loc));
    if (d.hint) li.append(el("div", "diag-hint", `hint: ${d.hint}`));
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
  await loadArtifacts();
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
      state.meta.validation = {
        ok: false,
        profile: name,
        errors: data.diagnostics.filter((d) => d.severity === "error").length,
        warnings: data.diagnostics.filter((d) => d.severity === "warning").length,
        info: data.diagnostics.filter((d) => d.severity === "info").length,
        visible_tables: (state.meta.validation && state.meta.validation.visible_tables) || 0,
        diagnostics: data.diagnostics,
      };
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

// ── 启动 ────────────────────────────────────────────────────────────

async function init() {
  document.getElementById("refresh-btn").addEventListener("click", refresh);
  document.getElementById("profile-sel").addEventListener("change", refresh);
  document.getElementById("level-sel").addEventListener("change", refresh);
  document.getElementById("env-in").addEventListener("change", refresh);
  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      refresh();
    }
  });
  await refresh();
}

init();
