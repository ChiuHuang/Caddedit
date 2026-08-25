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
  return { proxy: "proxy", static: "static", other: "simple", raw: "raw" }[kind] || kind;
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
  $("#empty").hidden = routes.length > 0;

  for (const r of routes) {
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

$("#btn-reload").addEventListener("click", async () => {
  try {
    const r = await api("/api/reload", { method: "POST" });
    toast(r.ok ? "caddy reloaded" : `reload failed: ${r.error}`);
  } catch (e) {
    toast(e.message, { action: "dismiss" });
  }
});

const savedTheme = localStorage.getItem("caddedit-theme");
if (savedTheme) {
  document.documentElement.classList.toggle("mdui-theme-dark", savedTheme === "dark");
  document.documentElement.classList.toggle("mdui-theme-light", savedTheme === "light");
}
$("#btn-theme").addEventListener("click", () => {
  const el = document.documentElement;
  const dark = el.classList.contains("mdui-theme-dark");
  el.classList.toggle("mdui-theme-dark", !dark);
  el.classList.toggle("mdui-theme-light", dark);
  localStorage.setItem("caddedit-theme", dark ? "light" : "dark");
});

/* ---------- editor dialog ---------- */

let editingId = null;

async function openEditor(r) {
  editingId = r.id;
  $("#edit-id").textContent = r.id;
  $("#edit-error").textContent = "";
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
