/* Caddedit dashboard — vanilla JS + MDUI 2 web components. */
"use strict";

const $ = (s) => document.querySelector(s);

const ICONS = {
  edit: `<svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor"><path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"/></svg>`,
  trash: `<svg viewBox="0 0 24 24" width="20" height="20" fill="currentColor"><path d="M6 19a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/></svg>`,
};

let routes = [];
let authRequired = false;

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

let searchQuery = "";

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

function tlsLabel(tls) {
  if (!tls || tls.mode === "none") return "no tls";
  if (tls.mode === "internal") return "tls internal";
  if (tls.mode === "acme_email") return `acme (${tls.detail || ""})`;
  if (tls.mode === "dns") return `dns: ${tls.detail || "?"}`;
  if (tls.mode === "manual") return `cert/key`;
  return tls.detail || "custom";
}

function render() {
  const wrap = $("#routes");
  wrap.innerHTML = "";
  const visible = routes.filter((r) => routeMatches(r, searchQuery));
  $("#empty").hidden = visible.length > 0;
  $("#empty").textContent = routes.length === 0
    ? "No routes yet."
    : `No routes match “${searchQuery}”.`;

  for (const r of visible) {
    const card = document.createElement("div");
    card.className = "route-card";

    const sw = document.createElement("mdui-switch");
    sw.checked = r.status === "on";
    sw.setAttribute("aria-label", "toggle route");

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

    if (r.watch_log || (r.details || []).length) {
      const bits = [];
      if (r.watch_log) bits.push("request_watch_log");
      const det = document.createElement("div");
      det.className = "route-meta mono";
      det.style.opacity = "0.75";
      det.textContent =
        bits.concat(r.details.slice(0, 4)).join("  ·  ") +
        (r.details.length > 4 ? `  ·  +${r.details.length - 4} more` : "");
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

/* ---------- reload / theme ---------- */

$("#btn-refresh").addEventListener("click", async () => {
  await loadAll();
  toast(`loaded ${routes.length} routes`);
});

$("#search").addEventListener("input", (e) => {
  searchQuery = e.target.value.trim().toLowerCase();
  render();
});

$("#btn-reload").addEventListener("click", async () => {
  try {
    const r = await api("/api/reload", { method: "POST" });
    toast(r.ok ? "caddy reloaded" : `reload failed: ${r.error}`);
  } catch (e) {
    toast(e.message, { action: "dismiss" });
  }
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

/* quick toggle: dark <-> light */
$("#btn-theme").addEventListener("click", () => {
  const dark =
    document.documentElement.classList.contains("mdui-theme-dark") ||
    (!document.documentElement.classList.contains("mdui-theme-light") &&
      localStorage.getItem("caddedit-theme") !== "light" &&
      localStorage.getItem("caddedit-theme") === "dark");
  applyTheme(dark ? "light" : "dark");
});

/* ---------- editor dialog ---------- */

let editingId = null;
let editingRoute = null;

function parsedHtml(r) {
  const rows = [];
  const chipColor = {
    proxy: "#26a69a", php: "#42a5f5", static: "#ffb74d",
    simple: "#90a4ae", other: "#90a4ae", raw: "#ef5350",
  }[kindLabel(r.kind)] || "#90a4ae";
  rows.push(`<div class="parsed-row"><b>Type</b><span class="chip" style="background:${chipColor}33;color:${chipColor}">${kindLabel(r.kind)}</span></div>`);
  rows.push(`<div class="parsed-row"><b>Domains</b>${(r.addresses.join(", ") || r.id)}</div>`);
  if (r.upstreams.length)
    rows.push(`<div class="parsed-row"><b>Upstream</b>${r.upstreams.join(", ")}</div>`);
  if (r.tls) rows.push(`<div class="parsed-row"><b>TLS</b>${tlsLabel(r.tls)}</div>`);
  if (r.watch_log)
    rows.push(`<div class="parsed-row"><b>Logging</b><span class="chip">request_watch_log</span></div>`);
  if ((r.details || []).length) {
    rows.push(`<div class="parsed-row"><b>Directives</b>${r.details.map((d) => `· ${d}`).join("<br>")}</div>`);
  }
  if (r.kind === "raw") {
    rows.push(`<div class="parsed-row" style="opacity:.7">This route uses syntax the structured parser doesn't fully model — edit the raw source on the left.</div>`);
  }
  return rows.join("");
}

async function openEditor(r) {
  editingId = r.id;
  editingRoute = r;
  $("#edit-id").textContent = r.id;
  $("#edit-error").textContent = "";
  $("#parsed-body").innerHTML = parsedHtml(r);
  const field = $("#edit-content");
  field.loading = true;
  $("#dlg-edit").open = true;
  try {
    const data = await api(`/api/vhosts/${encodeURIComponent(r.id)}/raw`);
    field.value = data.content;
  } catch (e) {
    $("#dlg-edit").open = false;
    toast(e.message, { action: "dismiss" });
    return;
  }
  field.loading = false;
}

$("#edit-cancel").addEventListener("click", () => ($("#dlg-edit").open = false));
$("#edit-save").addEventListener("click", saveEditor);
async function saveEditor() {
  const btn = $("#edit-save");
  btn.loading = true;
  try {
    await api(`/api/vhosts/${encodeURIComponent(editingId)}/raw`, {
      method: "PUT",
      body: JSON.stringify({ content: $("#edit-content").value, reload: true }),
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

const TLS_SNIPPETS = {
  none: "",
  internal: "",
  internal_explicit: "\n\ttls internal",
  cloudflare: `\n\ttls {\n\t\tdns cloudflare {$CF_API_TOKEN}\n\t}`,
};

$("#fab-new").addEventListener("click", () => {
  $("#new-domains").value = "";
  $("#new-upstream").value = "";
  $("#new-tls").value = "internal";
  $("#new-watch-log").checked = false;
  $("#new-error").textContent = "";
  $("#dlg-new").open = true;
});
$("#new-cancel").addEventListener("click", () => ($("#dlg-new").open = false));
$("#new-create").addEventListener("click", createRoute);
async function createRoute() {
  const payload = {
    domains: $("#new-domains").value,
    upstream: $("#new-upstream").value.trim(),
    tls: $("#new-tls").value,
    watch_log: $("#new-watch-log").checked,
  };
  try {
    await api("/api/vhosts", { method: "POST", body: JSON.stringify(payload) });
    $("#dlg-new").open = false;
    toast("route created");
    loadAll();
  } catch (e) {
    $("#new-error").textContent = e.body?.error || e.message;
  }
}

/* ---------- boot ---------- */

(async () => {
  const unlocked = await loadStatus().catch(() => false);
  if (unlocked) await loadAll();
})();
