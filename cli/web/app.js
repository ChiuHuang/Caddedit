/* Caddedit dashboard — vanilla JS + MDUI 2 web components. */
"use strict";

const $ = (s) => document.querySelector(s);

const ICONS = {
  edit: `<svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor"><path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"/></svg>`,
  trash: `<svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor"><path d="M6 19a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/></svg>`,
  gear: `<svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor"><path d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 0 0 .12-.61l-1.92-3.32a.488.488 0 0 0-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.484.484 0 0 0-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 0 0-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6A3.6 3.6 0 1 1 12 8.4a3.6 3.6 0 0 1 0 7.2z"/></svg>`,
};

let routes = [];
let authRequired = false;
let searchQuery = "";

function toast(msg, action) {
  mdui.snackbar({ message: msg, placement: "bottom", timeout: 4000, ...action });
}

async function api(path, opts = {}) {
  const res = await fetch(path, {
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    ...opts,
  });
  if (res.status === 401 && path !== "/api/login") {
    openLogin();
    throw new Error("locked");
  }
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw Object.assign(new Error(body.error || res.statusText), { body });
  return body;
}

/* ---------- rendering ---------- */

function kindLabel(kind) {
  return { proxy: "proxy", php: "php", static: "static", other: "simple", raw: "raw" }[kind] || kind;
}

function tlsLabel(tls) {
  if (!tls) return "no tls";
  if (tls.mode === "internal") return "tls internal";
  if (tls.mode === "acme_email") return `acme (${tls.detail || ""})`;
  if (tls.mode === "dns") return `dns: ${tls.detail || "?"}`;
  if (tls.mode === "manual") return "cert/key";
  return tls.detail || "custom";
}

function routeMatches(r, q) {
  if (!q) return true;
  const hay = [
    r.id,
    r.addresses.join(" "),
    kindLabel(r.kind),
    r.upstreams.join(" "),
    tlsLabel(r.tls),
    r.watch_log ? "request_watch_log" : "",
    (r.details || []).join(" "),
  ].join(" ").toLowerCase();
  return hay.includes(q);
}

function render() {
  const wrap = $("#routes");
  wrap.innerHTML = "";
  const visible = routes.filter((r) => routeMatches(r, searchQuery));
  $("#empty").hidden = visible.length > 0;
  $("#empty").textContent =
    routes.length === 0 ? "No routes yet." : `No routes match "${searchQuery}".`;

  for (const r of visible) {
    const card = document.createElement("div");
    card.className = "route-card";

    const sw = document.createElement("mdui-switch");
    sw.checked = r.status === "on";

    const main = document.createElement("div");
    main.className = "route-main";
    const domains = document.createElement("div");
    domains.className = "route-domains";
    domains.textContent = r.addresses.join(", ") || r.id;
    const meta = document.createElement("div");
    meta.className = "route-meta mono";
    meta.textContent = [
      kindLabel(r.kind),
      r.upstreams.length ? "-> " + r.upstreams.join(", ") : null,
      tlsLabel(r.tls),
    ].filter(Boolean).join("   ·   ");
    main.append(domains, meta);

    const bits = [];
    if (r.watch_log) bits.push("request_watch_log");
    for (const d of (r.details || []).slice(0, 4)) bits.push(d);
    if (bits.length) {
      const det = document.createElement("div");
      det.className = "route-meta mono";
      det.style.opacity = "0.75";
      det.textContent =
        bits.join("  ·  ") +
        ((r.details || []).length > 4 ? `  ·  +${r.details.length - 4} more` : "");
      main.append(det);
    }

    const actions = document.createElement("div");
    actions.className = "route-actions";

    const btnEdit = document.createElement("mdui-button-icon");
    btnEdit.innerHTML = ICONS.edit;
    btnEdit.addEventListener("click", () => openEditor(r));

    const btnDel = document.createElement("mdui-button-icon");
    btnDel.innerHTML = ICONS.trash;
    btnDel.style.color = "rgb(var(--mdui-color-error))";
    btnDel.addEventListener("click", () => removeRoute(r));

    actions.append(btnEdit, btnDel);
    card.append(sw, main, actions);

    sw.addEventListener("change", async () => {
      const target = sw.checked;
      try {
        await api(`/api/vhosts/${encodeURIComponent(r.id)}/toggle`, {
          method: "POST",
          body: JSON.stringify({ reload: true }),
        });
        r.status = target ? "on" : "off";
        toast(`${r.id} is ${target ? "on" : "off"}, caddy reloaded`);
      } catch (e) {
        sw.checked = !target;
        toast(`failed: ${e.message}`, { action: "dismiss" });
      }
    });

    wrap.append(card);
  }
}

async function loadAll() {
  try {
    routes = await api("/api/vhosts");
    render();
  } catch (e) {
    if (e.message !== "locked") toast(e.message, { action: "dismiss" });
  }
}

async function loadStatus() {
  const st = await api("/api/status");
  authRequired = st.auth_required;
  $("#config-path").textContent = st.config_path;
  if (st.auth_required && !st.authenticated) {
    openLogin();
    return false;
  }
  return true;
}

/* ---------- login ---------- */

function openLogin() {
  $("#login-error").textContent = "";
  $("#dlg-login").open = true;
  setTimeout(() => $("#login-password").focus(), 50);
}

$("#login-go").addEventListener("click", doLogin);
$("#login-password").addEventListener("keydown", (e) => {
  if (e.key === "Enter") doLogin();
});
async function doLogin() {
  try {
    await api("/api/login", {
      method: "POST",
      body: JSON.stringify({ password: $("#login-password").value }),
    });
    $("#dlg-login").open = false;
    $("#login-password").value = "";
    await loadStatus();
    await loadAll();
  } catch (e) {
    $("#login-error").textContent = e.message === "locked" ? "" : "wrong password";
  }
}

$("#btn-logout").addEventListener("click", async () => {
  await api("/api/logout", { method: "POST" }).catch(() => {});
  routes = [];
  render();
  if (authRequired) openLogin();
});

/* ---------- toolbar ---------- */

$("#btn-refresh").addEventListener("click", async () => {
  await loadAll();
  toast(`loaded ${routes.length} routes`);
});

$("#btn-reload").addEventListener("click", async () => {
  try {
    const r = await api("/api/reload", { method: "POST" });
    toast(r.ok ? "caddy reloaded" : `reload failed: ${r.error}`);
  } catch (e) {
    toast(e.message, { action: "dismiss" });
  }
});

$("#search").addEventListener("input", (e) => {
  searchQuery = e.target.value.trim().toLowerCase();
  render();
});

/* ---------- theme + settings ---------- */

const ACCENTS = [
  ["Purple", "#6750A4"],
  ["Blue", "#1565C0"],
  ["Teal", "#00897B"],
  ["Green", "#2E7D32"],
  ["Amber", "#B26A00"],
  ["Red", "#C62828"],
];

function applyTheme(mode) {
  mdui.setTheme(mode);
  localStorage.setItem("caddedit-theme", mode);
  const group = $("#set-theme");
  if (group) {
    for (const b of group.querySelectorAll("mdui-segmented-button")) {
      b.selected = b.value === mode;
    }
  }
}

function applyAccent(hex) {
  mdui.setColorScheme(hex);
  localStorage.setItem("caddedit-accent", hex);
  document.querySelectorAll(".swatch").forEach((s) => {
    s.classList.toggle("active", s.dataset.color === hex);
    s.style.background = s.dataset.color;
  });
}

$("#btn-settings").addEventListener("click", () => ($("#dlg-settings").open = true));
$("#settings-done").addEventListener("click", () => ($("#dlg-settings").open = false));

$("#set-theme").addEventListener("change", () => {
  const sel = $("#set-theme").querySelector("mdui-segmented-button[selected]");
  if (sel) applyTheme(sel.value);
});

(function initSettings() {
  const sw = $("#swatches");
  for (const [name, hex] of ACCENTS) {
    const b = document.createElement("button");
    b.className = "swatch";
    b.title = name;
    b.dataset.color = hex;
    b.addEventListener("click", () => applyAccent(hex));
    sw.append(b);
  }
  applyAccent(localStorage.getItem("caddedit-accent") || "#6750A4");
  applyTheme(localStorage.getItem("caddedit-theme") || "dark");
})();

$("#btn-theme").addEventListener("click", () => {
  const cur = localStorage.getItem("caddedit-theme") || "dark";
  applyTheme(cur === "dark" ? "light" : "dark");
});

/* ---------- CLI refresh token ---------- */

async function loadTokenStatus() {
  try {
    const s = await api("/api/auth/tokens/status");
    const el = $("#cli-token-status");
    if (s.has_refresh_token) {
      const d = new Date(s.refresh_created_at);
      el.textContent = `Refresh token exists (created ${d.toLocaleString()}). Active access tokens: ${s.active_access_tokens}.`;
      $("#cli-token-meta").textContent = s.refresh_created_by_ua ? `Created by UA: ${s.refresh_created_by_ua}` : "";
    } else {
      el.textContent = "No refresh token yet — generate one for CLI access.";
      $("#cli-token-meta").textContent = "";
    }
  } catch (e) {
    $("#cli-token-status").textContent = `Token status: ${e.message}`;
  }
}

$("#btn-settings").addEventListener("click", () => {
  loadTokenStatus();
  $("#cli-token-display").hidden = true;
  $("#btn-copy-refresh").hidden = true;
});

$("#btn-gen-refresh").addEventListener("click", async () => {
  const btn = $("#btn-gen-refresh");
  btn.loading = true;
  try {
    const r = await api("/api/auth/tokens/generate", { method: "POST", body: JSON.stringify({}) });
    const disp = $("#cli-token-display");
    disp.textContent = r.refresh_token;
    disp.hidden = false;
    $("#btn-copy-refresh").hidden = false;
    $("#cli-token-meta").textContent = "Copy now — this is shown once. Old token is now invalid.";
    toast("refresh token generated — old token invalidated");
    loadTokenStatus();
  } catch (e) {
    toast(e.body?.error || e.message, { action: "dismiss" });
  }
  btn.loading = false;
});

$("#btn-copy-refresh").addEventListener("click", async () => {
  const tok = $("#cli-token-display").textContent;
  try {
    await navigator.clipboard.writeText(tok);
    toast("copied to clipboard");
  } catch {
    toast("copy failed — select and copy manually", { action: "dismiss" });
  }
});

/* ---------- editor dialog ---------- */

let editingId = null;

function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  }[c]));
}

function parsedHtml(r) {
  const rows = [];
  const chipColor = {
    proxy: "#26a69a", php: "#42a5f5", static: "#ffb74d",
    simple: "#90a4ae", other: "#90a4ae", raw: "#ef5350",
  }[kindLabel(r.kind)] || "#90a4ae";
  rows.push(`<div class="parsed-row"><b>Type</b><span class="chip" style="background:${chipColor}33;color:${chipColor}">${esc(kindLabel(r.kind))}</span></div>`);
  rows.push(`<div class="parsed-row"><b>Domains</b>${esc(r.addresses.join(", ") || r.id)}</div>`);
  if (r.upstreams.length)
    rows.push(`<div class="parsed-row"><b>Upstream</b>${esc(r.upstreams.join(", "))}</div>`);
  if (r.tls) rows.push(`<div class="parsed-row"><b>TLS</b>${esc(tlsLabel(r.tls))}</div>`);
  if (r.watch_log)
    rows.push(`<div class="parsed-row"><b>Logging</b><span class="chip">request_watch_log</span></div>`);
  if ((r.details || []).length) {
    rows.push(`<div class="parsed-row"><b>Directives</b>${r.details.map((d) => `· ${esc(d)}`).join("<br>")}</div>`);
  }
  if (r.kind === "raw") {
    rows.push(`<div class="parsed-row" style="opacity:.7">This route uses syntax the structured parser does not fully model — prefer the Raw tab for edits.</div>`);
  }
  return rows.join("");
}

/* ---------- parsed editor: client-side site-block model ---------- */

let editModel = null;

function stripComments(line) {
  let inQuote = false, esc = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (esc) { esc = false; continue; }
    if (ch === "\\") { esc = true; continue; }
    if (ch === '"') { inQuote = !inQuote; continue; }
    if (ch === "#" && !inQuote) return line.slice(0, i);
  }
  return line;
}

function braceDelta(line) {
  const s = stripComments(line).replace(/"(?:\\.|[^"\\])*"/g, '""');
  let d = 0;
  for (const ch of s) {
    if (ch === "{") d++;
    else if (ch === "}") d--;
  }
  return d;
}

/// Locate header / watch_log / tls / simple reverse_proxy / everything else.
/// Everything else becomes editable directive entries: single-line directives
/// (type + args), directive blocks (raw text), and merged comment/blank runs.
const KNOWN_TYPES = [
  "import", "file_server", "redir", "respond", "encode", "header",
  "php_fastcgi", "root", "try_files", "request_body", "basic_auth",
  "log", "handle_path", "rewrite", "reverse_proxy", "tls",
];

/// Map every line belonging to a Caddyfile heredoc (the `<<MARKER` opener
/// line, its body and the closing marker line) to the closing line's index.
/// Heredoc content must never be split into separate directives or counted
/// for braces — CSS/HTML bodies are full of both.
function scanHeredocs(lines) {
  const closeAt = new Map();
  for (let i = 0; i < lines.length; i++) {
    if (closeAt.has(i)) continue;
    const m = lines[i].trim().match(/<<([A-Za-z0-9_]+)\s*$/);
    if (!m) continue;
    let end = lines.length - 1;
    for (let j = i + 1; j < lines.length; j++) {
      if (lines[j].trim() === m[1]) { end = j; break; }
    }
    for (let j = i; j <= end; j++) closeAt.set(j, end);
  }
  return closeAt;
}

function parseSiteBlock(text) {
  const lines = text.split("\n");
  const heredocEnd = scanHeredocs(lines);
  const p = {
    lines,
    headerIdx: -1, headerIndent: "", headerText: "", addrs: [],
    watchIdx: null, tls: null, rp: null,
    directives: [], closeIdx: -1,
  };
  let i = 0;
  while (i < lines.length) {
    const t = lines[i].trim();
    if (!t || t.startsWith("#")) { i++; continue; }
    break;
  }
  if (i >= lines.length) return p;
  p.headerIdx = i;
  p.headerIndent = (lines[i].match(/^\s*/) || [""])[0];
  let h = lines[i].trim();
  if (h.endsWith("{")) h = h.slice(0, -1).trim();
  p.headerText = h;
  p.addrs = h.split(/[,\s]+/).filter(Boolean);

  const consumeBlock = (start) => {
    let d = braceDelta(lines[start]), j = start;
    while (d > 0 && j + 1 < lines.length) {
      j++;
      if (!heredocEnd.has(j)) d += braceDelta(lines[j]);
    }
    return j;
  };

  i++;
  while (i < lines.length) {
    const t = lines[i].trim();
    const delta = braceDelta(lines[i]);
    const m = t.match(/^(\S+)\s*(.*)$/);
    const kw = m ? m[1] : "";
    const rest = m ? m[2] : "";
    if (!t || t.startsWith("#")) {
      const start = i;
      while (i + 1 < lines.length) {
        const nt = lines[i + 1].trim();
        if (!nt || nt.startsWith("#")) i++;
        else break;
      }
      p.directives.push({ kind: "raw", type: "raw", start, end: i, raw: lines.slice(start, i + 1).join("\n") });
      i++;
      continue;
    }
    const hEnd = heredocEnd.get(i);
    if (hEnd != null && hEnd > i) {
      p.directives.push({
        kind: "block", type: kw || "raw", heredoc: true,
        start: i, end: hEnd, raw: lines.slice(i, hEnd + 1).join("\n"),
      });
      i = hEnd + 1;
      continue;
    }
    if (hEnd != null) { i++; continue; }
    if (delta < 0) { p.closeIdx = i; break; }
    if (kw === "import" && rest === "request_watch_log") {
      p.watchIdx = i;
      i++;
      continue;
    }
    if (kw === "tls" && delta > 0) {
      const end = consumeBlock(i);
      p.tls = { start: i, end, block: true, args: "", raw: lines.slice(i, end + 1).join("\n") };
      i = end + 1;
      continue;
    }
    if (kw === "tls") {
      p.tls = { start: i, end: i, block: false, args: rest, raw: lines[i] };
      i++;
      continue;
    }
    if (kw === "reverse_proxy" && delta <= 0 && rest && !rest.includes("{")) {
      p.rp = { start: i, target: rest };
      i++;
      continue;
    }
    if (delta > 0) {
      const end = consumeBlock(i);
      p.directives.push({ kind: "block", type: kw, start: i, end, raw: lines.slice(i, end + 1).join("\n") });
      i = end + 1;
      continue;
    }
    if (KNOWN_TYPES.includes(kw)) {
      p.directives.push({ kind: "simple", type: kw, args: rest, start: i, end: i });
    } else {
      p.directives.push({ kind: "raw", type: "raw", start: i, end: i, raw: lines[i] });
    }
    i++;
  }
  return p;
}

function currentParsedState() {
  return {
    addrs: $("#pe-domains").value.split(/[,\s]+/).map((s) => s.trim()).filter(Boolean),
    upstream: $("#pe-upstream").value.trim(),
    mode: $("#pe-tls").value,
    detail: $("#pe-tls-detail").value.trim(),
    raw: $("#pe-tls-raw").value,
    watch: $("#pe-watch-log").checked,
  };
}

function updateTlsFields() {
  const m = $("#pe-tls").value;
  $("#pe-tls-detail-wrap").hidden = !(m === "acme" || m === "manual");
  $("#pe-tls-detail").label = m === "manual" ? "Certificate and key paths" : "ACME email";
  $("#pe-tls-raw-wrap").hidden = !(m === "cloudflare" || m === "custom");
  if (m === "cloudflare" && !$("#pe-tls-raw").value.trim()) {
    $("#pe-tls-raw").value = "tls {\n\tdns cloudflare {$CF_API_TOKEN}\n}";
  }
}

/// Replacement lines for the tls section under the current GUI state.
/// null = keep whatever is there now; [] = remove it.
function tlsReplacement(st) {
  switch (st.mode) {
    case "none": return [];
    case "internal": return ["\ttls internal"];
    case "acme":
    case "manual":
      return st.detail ? ["\ttls " + st.detail] : null;
    case "cloudflare":
    case "custom": {
      const raw = st.raw.replace(/\s+$/, "");
      if (!raw.trim()) return null;
      return raw.split("\n").map((l) => (/^\s/.test(l) ? l : "\t" + l));
    }
    default: return null;
  }
}

/// Replacement lines for one directive row. Empty array = remove it.
function directiveReplacement(d) {
  if (d.type === "raw") {
    const raw = (d.raw || "").replace(/\s+$/, "");
    if (!raw.trim()) return [];
    return raw.split("\n").map((l) => (/^\s/.test(l) ? l : "\t" + l));
  }
  const args = (d.args || "").trim();
  return ["\t" + d.type + (args ? " " + args : "")];
}

function directiveChanged(d) {
  if (!d.orig || d.fresh || d.deleted) return true;
  if (d.type === "raw") return (d.raw || "").replace(/\s+$/, "") !== (d.orig.raw || "").replace(/\s+$/, "");
  return d.type !== d.orig.type || (d.args || "").trim() !== (d.orig.args || "").trim();
}

function directivesDirty() {
  return editModel.directives.some((d) => d.fresh || d.deleted || directiveChanged(d));
}

/// Surgical rebuild: only lines touched through the GUI change; everything
/// else (comments, blank lines, complex blocks) stays byte-for-byte.
function buildParsedSource() {
  const p = editModel.p;
  const st = currentParsedState();
  const init = editModel.initial;
  const tlsTouched =
    st.mode !== init.mode || st.detail !== init.detail || st.raw.trim() !== init.raw.trim();

  const inserts = [];
  if (st.watch && p.watchIdx == null) inserts.push("\timport request_watch_log");
  if (st.upstream && !p.rp) inserts.push("\treverse_proxy " + st.upstream);
  if (!p.tls) {
    const rep = tlsReplacement(st);
    if (rep && rep.length) inserts.push(...rep);
  }
  const fresh = editModel.directives.filter((d) => d.fresh && !d.deleted);

  const byStart = new Map();
  for (const d of editModel.directives) {
    if (d.start >= 0) byStart.set(d.start, d);
  }

  const out = [];
  for (let i = 0; i < p.lines.length; i++) {
    if (i === p.headerIdx) {
      const header = p.headerIndent + st.addrs.join(", ") + " {";
      out.push(header === p.lines[i] ? p.lines[i] : header);
      out.push(...inserts.splice(0));
      continue;
    }
    if (p.watchIdx === i) {
      if (st.watch) out.push(p.lines[i]);
      continue;
    }
    if (p.rp && p.rp.start === i) {
      if (st.upstream) {
        const ind = (p.lines[i].match(/^\s*/) || [""])[0];
        out.push(st.upstream === p.rp.target ? p.lines[i] : ind + "reverse_proxy " + st.upstream);
      }
      continue;
    }
    if (p.tls && i >= p.tls.start && i <= p.tls.end) {
      if (i === p.tls.end) {
        if (st.mode === "none") {
          /* dropped */
        } else if (!tlsTouched) {
          out.push(p.lines.slice(p.tls.start, p.tls.end + 1).join("\n"));
        } else {
          const rep = tlsReplacement(st);
          if (rep) out.push(rep.join("\n"));
          else out.push(p.lines.slice(p.tls.start, p.tls.end + 1).join("\n"));
        }
      }
      continue;
    }
    if (byStart.has(i)) {
      const d = byStart.get(i);
      if (d.deleted) { i = d.end; continue; }
      if (directiveChanged(d)) {
        out.push(...directiveReplacement(d));
        i = d.end;
        continue;
      }
      /* unchanged: fall through, emit original lines verbatim */
    }
    if (i === p.closeIdx) {
      for (const d of fresh) out.push(...directiveReplacement(d));
    }
    out.push(p.lines[i]);
  }
  if (p.closeIdx < 0) {
    for (const d of fresh) out.push(...directiveReplacement(d));
  }
  return out.join("\n");
}

function parsedDirty() {
  if (!editModel || !editModel.initial) return false;
  const st = currentParsedState();
  const i = editModel.initial;
  return (
    st.addrs.join(", ") !== i.domains ||
    st.upstream !== i.upstream ||
    st.mode !== i.mode ||
    st.detail !== i.detail ||
    st.raw.trim() !== i.raw.trim() ||
    st.watch !== i.watch ||
    directivesDirty()
  );
}

function directiveRow(d) {
  const row = document.createElement("div");
  row.className = "dir-row";
  if (d.kind === "simple" && d.type !== "raw") {
    const sel = document.createElement("mdui-select");
    sel.variant = "outlined";
    sel.label = "type";
    for (const t of KNOWN_TYPES) {
      const mi = document.createElement("mdui-menu-item");
      mi.value = t;
      mi.textContent = t;
      sel.append(mi);
    }
    sel.value = d.type;
    sel.addEventListener("change", () => {
      d.type = sel.value;
      renderDirectives();
    });
    row.append(sel);
    const inp = document.createElement("mdui-text-field");
    inp.variant = "outlined";
    inp.label = "arguments";
    inp.value = d.args || "";
    inp.setAttribute("autocomplete", "off");
    inp.setAttribute("spellcheck", "false");
    inp.addEventListener("input", () => (d.args = inp.value));
    row.append(inp);
  } else {
    const ta = document.createElement("mdui-text-field");
    ta.className = "mono";
    ta.variant = "outlined";
    ta.label = d.kind === "block"
      ? (d.heredoc ? `${d.type} (heredoc — raw)` : `${d.type} (block — raw)`)
      : "raw";
    ta.value = d.raw || "";
    ta.setAttribute("autosize", "");
    ta.setAttribute("min-rows", "1");
    ta.setAttribute("max-rows", "12");
    ta.setAttribute("autocomplete", "off");
    ta.setAttribute("spellcheck", "false");
    ta.addEventListener("input", () => (d.raw = ta.value));
    row.append(ta);
  }
  const del = document.createElement("mdui-button-icon");
  del.innerHTML = ICONS.trash;
  del.addEventListener("click", () => {
    d.deleted = true;
    renderDirectives();
  });
  row.append(del);
  return row;
}

function renderDirectives() {
  const wrap = $("#pe-directives");
  wrap.innerHTML = "";
  for (const d of editModel.directives) wrap.append(directiveRow(d));
  $("#pe-directives-wrap").hidden = editModel.directives.length === 0;
}

function hydrateParsedEditor(r, raw) {
  const p = parseSiteBlock(raw);
  editModel = { raw, p, initial: null, directives: [] };
  $("#parsed-summary").innerHTML = parsedHtml(r);

  const upstream = p.rp ? p.rp.target : "";
  $("#pe-domains").value = p.addrs.join(", ");
  $("#pe-upstream").value = upstream;

  let mode = "none", detail = "", tlsRaw = "";
  if (p.tls && !p.tls.block) {
    const args = p.tls.args.trim();
    if (args === "internal") mode = "internal";
    else if (/^\S+@\S+$/.test(args)) { mode = "acme"; detail = args; }
    else if (/\s/.test(args)) { mode = "manual"; detail = args; }
    else { mode = "custom"; tlsRaw = args; }
  } else if (p.tls) {
    tlsRaw = p.tls.raw;
    mode = /dns\s+cloudflare\b/.test(tlsRaw) ? "cloudflare" : "custom";
  }
  $("#pe-tls").value = mode;
  $("#pe-tls-detail").value = detail;
  $("#pe-tls-raw").value = tlsRaw;
  $("#pe-watch-log").checked = p.watchIdx != null;
  updateTlsFields();

  editModel.directives = p.directives.map((d) => ({
    ...d,
    deleted: false,
    fresh: false,
    orig: { type: d.type, args: d.args || "", raw: d.raw || "" },
  }));
  renderDirectives();

  const notes = [];
  if (p.headerIdx < 0) notes.push("No site block header found — use the Raw tab.");
  if (p.directives.some((d) => d.heredoc))
    notes.push("Heredoc bodies are kept verbatim as single blocks — edit them as raw text.");
  if (r.kind === "proxy" && !p.rp && r.upstreams.length)
    notes.push(
      "Complex reverse_proxy block(s) are preserved below; the upstream field only controls a simple `reverse_proxy <target>` line."
    );
  if (r.kind === "raw")
    notes.push("The structured parser can't fully model this route — prefer the Raw tab.");
  $("#pe-note").textContent = notes.join(" ");

  editModel.initial = {
    domains: p.addrs.join(", "),
    upstream,
    mode, detail, raw: tlsRaw,
    watch: p.watchIdx != null,
  };
}

function setEditTab(v) {
  $("#edit-parsed-pane").hidden = v !== "parsed";
  $("#edit-raw-pane").hidden = v !== "raw";
}

async function openEditor(r) {
  editingId = r.id;
  $("#edit-id").textContent = r.id;
  $("#edit-error").textContent = "";
  $("#edit-tabs").value = "parsed";
  setEditTab("parsed");
  const field = $("#edit-content");
  field.loading = true;
  $("#dlg-edit").open = true;
  let raw;
  try {
    const data = await api(`/api/vhosts/${encodeURIComponent(r.id)}/raw`);
    raw = data.content;
  } catch (e) {
    $("#dlg-edit").open = false;
    toast(e.message, { action: "dismiss" });
    return;
  }
  field.loading = false;
  field.value = raw;
  hydrateParsedEditor(r, raw);
}

$("#edit-cancel").addEventListener("click", () => ($("#dlg-edit").open = false));
$("#edit-save").addEventListener("click", saveEditor);

function syncRawFromParsed() {
  if (parsedDirty()) $("#edit-content").value = buildParsedSource();
}

$("#edit-tabs").addEventListener("change", () => {
  const v = $("#edit-tabs").value;
  if (v === "raw") syncRawFromParsed();
  setEditTab(v);
});

$("#pe-tls").addEventListener("change", updateTlsFields);

$("#pe-add-dir").addEventListener("click", () => {
  editModel.directives.push({
    kind: "simple", type: "respond", args: "",
    start: -1, end: -1, deleted: false, fresh: true, orig: null,
  });
  renderDirectives();
});

$("#pe-goto-raw").addEventListener("click", () => {
  syncRawFromParsed();
  $("#edit-tabs").value = "raw";
  setEditTab("raw");
});

$("#pe-reload").addEventListener("click", async () => {
  try {
    const res = await api("/api/reload", { method: "POST" });
    toast(res.ok ? "caddy reloaded" : `reload failed: ${res.error}`);
  } catch (e) {
    toast(e.message, { action: "dismiss" });
  }
});

async function saveEditor() {
  const btn = $("#edit-save");
  let content = $("#edit-content").value;
  if ($("#edit-tabs").value === "parsed") {
    if (!currentParsedState().addrs.length) {
      $("#edit-error").textContent = "at least one domain is required";
      return;
    }
    content = buildParsedSource();
    $("#edit-content").value = content;
  }
  btn.loading = true;
  try {
    await api(`/api/vhosts/${encodeURIComponent(editingId)}/raw`, {
      method: "PUT",
      body: JSON.stringify({ content, reload: true }),
    });
    $("#edit-error").textContent = "";
    $("#dlg-edit").open = false;
    toast("saved, caddy reloaded");
    loadAll();
  } catch (e) {
    $("#edit-error").textContent = e.body?.error || e.message;
  }
  btn.loading = false;
}

/* ---------- delete ---------- */

async function removeRoute(r) {
  const ok = await mdui.confirm({
    headline: `Remove ${r.id}?`,
    description: "The file moves to the backups folder — recoverable.",
    confirmText: "Remove",
    cancelText: "Keep",
  });
  if (!ok) return;
  try {
    await api(`/api/vhosts/${encodeURIComponent(r.id)}?reload=true`, { method: "DELETE" });
    toast(`${r.id} removed`);
    loadAll();
  } catch (e) {
    toast(e.message, { action: "dismiss" });
  }
}

/* ---------- create ---------- */

$("#fab-new").addEventListener("click", () => {
  $("#new-tabs").value = "parsed";
  $("#new-parsed-pane").hidden = false;
  $("#new-raw-pane").hidden = true;
  $("#new-domains").value = "";
  $("#new-upstream").value = "";
  $("#new-tls").value = "internal";
  $("#new-watch-log").checked = false;
  $("#new-raw").value = "";
  $("#new-error").textContent = "";
  $("#dlg-new").open = true;
});
$("#new-cancel").addEventListener("click", () => ($("#dlg-new").open = false));
$("#new-tabs").addEventListener("change", () => {
  const v = $("#new-tabs").value;
  $("#new-parsed-pane").hidden = v !== "parsed";
  $("#new-raw-pane").hidden = v !== "raw";
});
$("#new-create").addEventListener("click", createRoute);
async function createRoute() {
  let payload;
  if ($("#new-tabs").value === "raw") {
    const source = $("#new-raw").value;
    if (!source.trim()) {
      $("#new-error").textContent = "site block source is required";
      return;
    }
    payload = { source };
  } else {
    payload = {
      domains: $("#new-domains").value,
      upstream: $("#new-upstream").value.trim(),
      tls: $("#new-tls").value,
      watch_log: $("#new-watch-log").checked,
    };
  }
  try {
    await api("/api/vhosts", { method: "POST", body: JSON.stringify(payload) });
    $("#dlg-new").open = false;
    toast("route created");
    loadAll();
  } catch (e) {
    $("#new-error").textContent = e.body?.error || e.message;
  }
}

/* ---------- plugins (local, client-side scripts) ---------- */

const PLUGIN_KEY = "caddedit-plugins";
const pluginRegistry = [];

function pluginList() {
  try {
    return JSON.parse(localStorage.getItem(PLUGIN_KEY) || "[]");
  } catch {
    return [];
  }
}

function savePluginList(list) {
  localStorage.setItem(PLUGIN_KEY, JSON.stringify(list));
}

/* The API surface plugins get. A plugin script calls:
     caddedit.registerPlugin({ id, name, render(el, caddedit) })
   `el` is its panel container inside the Plugins dialog. */
window.caddedit = {
  registerPlugin(def) {
    if (!def || !def.id) return;
    const i = pluginRegistry.findIndex((p) => p.id === def.id);
    if (i >= 0) pluginRegistry[i] = def;
    else pluginRegistry.push(def);
    if ($("#dlg-plugins").open) renderPluginPanels();
  },
  api,
  toast,
  refresh: () => loadAll(),
  get routes() {
    return routes;
  },
};

function loadPluginScripts() {
  for (const p of pluginList()) {
    if (!p.enabled) continue;
    const s = document.createElement("script");
    s.src = p.url;
    s.async = false;
    s.onerror = () => toast(`plugin failed to load: ${p.url}`, { action: "dismiss" });
    document.head.append(s);
  }
}

function renderPluginPanels() {
  const wrap = $("#plugin-panels");
  wrap.innerHTML = "";
  $("#plugins-empty").hidden = pluginRegistry.length > 0;
  for (const def of pluginRegistry) {
    const card = document.createElement("div");
    card.className = "plugin-card";

    const head = document.createElement("div");
    head.style.cssText = "display:flex;align-items:center;gap:.25rem;margin-bottom:.5rem;";
    const title = document.createElement("div");
    title.style.flex = "1";
    title.style.fontWeight = "600";
    title.textContent = def.name || def.id;
    head.append(title);

    const body = document.createElement("div");
    let settingsBox = null;
    if (typeof def.renderSettings === "function") {
      const gear = document.createElement("mdui-button-icon");
      gear.innerHTML = ICONS.gear;
      gear.addEventListener("click", () => {
        if (!settingsBox) {
          settingsBox = document.createElement("div");
          settingsBox.style.cssText =
            "margin-top:.75rem;border-top:1px solid rgb(var(--mdui-color-outline-variant));padding-top:.75rem;";
          card.append(settingsBox);
          try {
            def.renderSettings(settingsBox, window.caddedit);
          } catch (e) {
            settingsBox.textContent = `plugin settings error: ${e.message}`;
          }
        }
        settingsBox.hidden = !settingsBox.hidden;
      });
      const tip = document.createElement("mdui-tooltip");
      tip.setAttribute("content", "Plugin settings");
      tip.append(gear);
      head.append(tip);
    }

    card.append(head, body);
    try {
      if (typeof def.render === "function") def.render(body, window.caddedit);
      else body.textContent = "plugin has no render()";
    } catch (e) {
      body.textContent = `plugin error: ${e.message}`;
    }
    wrap.append(card);
  }
}

function renderPluginSettings() {
  const wrap = $("#plugin-list");
  wrap.innerHTML = "";
  const list = pluginList();
  if (!list.length) {
    const empty = document.createElement("div");
    empty.className = "parsed-row";
    empty.style.opacity = ".6";
    empty.textContent = "No plugins added yet.";
    wrap.append(empty);
    return;
  }
  list.forEach((p, i) => {
    const row = document.createElement("div");
    row.className = "plugin-entry";
    const sw = document.createElement("mdui-switch");
    sw.checked = p.enabled;
    sw.addEventListener("change", () => {
      const l = pluginList();
      l[i].enabled = sw.checked;
      savePluginList(l);
      location.reload();
    });
    const url = document.createElement("div");
    url.className = "url mono";
    url.textContent = p.url;
    const del = document.createElement("mdui-button-icon");
    del.innerHTML = ICONS.trash;
    del.addEventListener("click", () => {
      const l = pluginList();
      l.splice(i, 1);
      savePluginList(l);
      location.reload();
    });
    row.append(sw, url, del);
    wrap.append(row);
  });
}

$("#btn-plugins").addEventListener("click", () => {
  renderPluginPanels();
  $("#dlg-plugins").open = true;
});
$("#plugins-close").addEventListener("click", () => ($("#dlg-plugins").open = false));
$("#plugin-add").addEventListener("click", () => {
  const field = $("#plugin-url");
  const url = field.value.trim();
  if (!url) return;
  const list = pluginList();
  if (list.some((p) => p.url === url)) {
    toast("plugin already added");
    return;
  }
  list.push({ url, enabled: true });
  savePluginList(list);
  location.reload();
});

/* ---------- self-update ---------- */

function setUpdText(html) {
  $("#upd-status").innerHTML = html;
}

const UPD_STAGES = {
  checking: "checking GitHub releases…",
  downloading: "downloading…",
  installing: "installing…",
  restarting: "restarting service…",
};

function setUpdBar(st) {
  const bar = $("#upd-bar");
  const active = st && st.running;
  bar.hidden = !active;
  if (!active) return;
  const secs = st.started_at ? Math.round((Date.now() - st.started_at) / 1000) : 0;
  setUpdText(
    `<b>${UPD_STAGES[st.stage] || st.stage}</b>${secs ? ` — ${secs}s` : ""}`
  );
}

let updPolling = false;

async function pollUpdateStatus() {
  if (updPolling) return;
  updPolling = true;
  try {
    while (true) {
      let st;
      try {
        st = await api("/api/update/status");
      } catch (_) {
        await new Promise((r) => setTimeout(r, 1500)); // service restarting
        continue;
      }
      setUpdBar(st);
      if (st.stage === "done") {
        updPolling = false;
        $("#btn-upd-apply").hidden = true;
        if (st.target) {
          setUpdText(`installed v${st.target} — waiting for restart…`);
          for (let i = 0; i < 30; i++) {
            await new Promise((r) => setTimeout(r, 1500));
            try {
              const res = await fetch("/api/status", { credentials: "same-origin" });
              if (!res.ok) continue;
              const body = await res.json();
              if (body.version === st.target) {
                toast(`updated to v${body.version}`);
                location.reload();
                return;
              }
            } catch (_) { /* still restarting */ }
          }
          setUpdText("restart timed out — check systemctl status caddedit-dashboard");
        } else {
          setUpdText(st.message || "up to date");
        }
        return;
      }
      if (st.stage === "error") {
        updPolling = false;
        $("#btn-upd-apply").hidden = true;
        setUpdText(`update failed: ${st.message}`);
        return;
      }
      await new Promise((r) => setTimeout(r, 1000));
    }
  } finally {
    updPolling = false;
  }
}

function updChannel() {
  return $("#upd-nightly") && $("#upd-nightly").checked ? "nightly" : "stable";
}

function md2html(md) {
  return md
    .replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;")
    .replace(/^### (.*)$/gm, "<b>$1</b>")
    .replace(/^## (.*)$/gm, "<b>$1</b>")
    .replace(/\*\*(.*?)\*\*/g, "<b>$1</b>")
    .replace(/`([^`]+)`/g, '<code style="background:rgb(var(--mdui-color-surface-container-high));padding:0 4px;border-radius:4px;">$1</code>')
    .replace(/\n/g, "<br>");
}

function setUpdNotes(r) {
  const el = $("#upd-notes");
  if (!el) return;
  if (r && r.notes) {
    let txt = r.notes;
    if (r.published_at) {
      try {
        const d = new Date(r.published_at);
        txt += `\n\n— ${r.channel} ${r.latest} · ${d.toLocaleString()}`;
      } catch {}
    }
    el.innerHTML = md2html(txt);
    el.style.display = "block";
  } else {
    el.textContent = "";
    el.style.display = "none";
  }
}

async function checkForUpdates(silent) {
  if (!silent) {
    setUpdText("checking...");
    const n = $("#upd-notes");
    if (n) { n.textContent = ""; n.style.display = "none"; }
  }
  try {
    const r = await api("/api/update/check?channel=" + updChannel());
    document.querySelectorAll(".cur-version").forEach((el) => (el.textContent = r.current));
    if (!r.supported) {
      setUpdText(`v${r.current} — auto-update unsupported here`);
      setUpdNotes(r);
      return;
    }
    if (r.error) {
      setUpdText(`v${r.current} — check failed: ${r.error}`);
      setUpdNotes(r);
      return;
    }
    if (r.up_to_date) {
      setUpdText(`v${r.current} — up to date`);
      $("#btn-upd-apply").hidden = true;
      setUpdNotes(r);
      return;
    }
    const latestLabel = r.latest === "nightly" ? "nightly" : `v${r.latest}`;
    setUpdText(
      `<b style="color:rgb(var(--mdui-color-primary))">${latestLabel} available</b> (installed: v${r.current})`
    );
    setUpdNotes(r);
    const btn = $("#btn-upd-apply");
    btn.hidden = false;
    btn.loading = false;
    btn.textContent = r.latest === "nightly" ? `Update to nightly` : `Update to v${r.latest}`;
    btn.dataset.version = r.latest;
  } catch (e) {
    if (!silent) setUpdText(`check failed: ${e.message}`);
  }
}

$("#btn-upd-check").addEventListener("click", () => checkForUpdates(false));

(function initUpdChannel() {
  const cb = $("#upd-nightly");
  if (!cb) return;
  const saved = localStorage.getItem("caddedit-update-channel");
  if (saved === "nightly") cb.checked = true;
  cb.addEventListener("change", () => {
    localStorage.setItem("caddedit-update-channel", updChannel());
    checkForUpdates(false);
  });
})();

$("#btn-upd-apply").addEventListener("click", async () => {
  const btn = $("#btn-upd-apply");
  const target = btn.dataset.version;
  btn.hidden = true;
  const ch = updChannel();
  setUpdText(`starting update to ${ch === "nightly" ? "nightly " : "v"}${target}…`);
  try {
    await api("/api/update?channel=" + ch, { method: "POST" });
  } catch (e) {
    setUpdText(`update failed: ${e.body?.error || e.message}`);
    return;
  }
  pollUpdateStatus();
});

/* ---------- boot ---------- */

(async () => {
  checkForUpdates(true);
  api("/api/update/status")
    .then((st) => {
      if (st.running || (st.stage && st.stage !== "idle")) {
        $("#btn-upd-apply").hidden = true;
        pollUpdateStatus();
      }
    })
    .catch(() => {});
  renderPluginSettings();
  loadPluginScripts();
  const unlocked = await loadStatus().catch(() => false);
  if (unlocked) await loadAll();
})();
