//! `caddedit serve` — embedded MDUI dashboard over a small JSON API.

use crate::config::Paths;
use crate::{caddy, fsutil, vhost};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

const COOKIE: &str = "caddedit_session";

#[derive(RustEmbed)]
#[folder = "web/"]
struct Assets;

pub struct AppState {
    paths: Paths,
    password: Option<String>,
    sessions: Mutex<HashSet<String>>,
}

type SharedState = Arc<AppState>;

/* ---------- helpers ---------- */

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn session_token(headers: &HeaderMap) -> Option<String> {
    for value in headers.get_all(header::COOKIE) {
        let Ok(s) = value.to_str() else { continue };
        for pair in s.split(';') {
            if let Some(tok) = pair.trim().strip_prefix(COOKIE) {
                let tok = tok.strip_prefix('=')?;
                return Some(tok.to_string());
            }
        }
    }
    None
}

fn authenticated(st: &AppState, headers: &HeaderMap) -> bool {
    if st.password.is_none() {
        return true;
    }
    match session_token(headers) {
        Some(tok) => st
            .sessions
            .lock()
            .map(|m| m.contains(&tok))
            .unwrap_or(false),
        None => false,
    }
}

async fn auth_mw(State(st): State<SharedState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let open = path == "/api/status" || path == "/api/login";
    if open || st.password.is_none() {
        return next.run(req).await;
    }
    if authenticated(&st, req.headers()) {
        next.run(req).await
    } else {
        err(StatusCode::UNAUTHORIZED, "locked")
    }
}

/* ---------- assets ---------- */

async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    serve_asset(path)
}

fn serve_asset(path: &str) -> Response {
    match Assets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                file.data,
            )
                .into_response()
        }
        None => match Assets::get("index.html") {
            Some(fallback) => (
                [(header::CONTENT_TYPE, "text/html".to_string())],
                fallback.data,
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
    }
}

/* ---------- request bodies ---------- */

#[derive(Deserialize)]
struct LoginReq {
    password: String,
}

#[derive(Deserialize)]
struct SaveReq {
    content: String,
    #[serde(default)]
    reload: bool,
}

#[derive(Deserialize)]
struct ToggleReq {
    #[serde(default = "yes")]
    reload: bool,
}
fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct CreateReq {
    #[serde(default)]
    domains: Option<String>,
    #[serde(default)]
    upstream: String,
    #[serde(default = "default_tls")]
    tls: String,
    #[serde(default)]
    watch_log: bool,
    /// When present, the request creates the route from a raw site block
    /// instead of the structured fields (dashboard "Raw" create tab).
    #[serde(default)]
    source: Option<String>,
}
fn default_tls() -> String {
    "internal".into()
}

/* ---------- handlers ---------- */

async fn status(State(st): State<SharedState>, headers: HeaderMap) -> Response {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "config_path": st.paths.caddyfile.display().to_string(),
        "vhosts_dir": st.paths.vhosts_dir.display().to_string(),
        "caddy_available": caddy::caddy_available(),
        "auth_required": st.password.is_some(),
        "authenticated": authenticated(&st, &headers),
    }))
    .into_response()
}

async fn login(State(st): State<SharedState>, Json(req): Json<LoginReq>) -> Response {
    let expected = match &st.password {
        Some(p) => p,
        None => return Json(json!({"ok": true})).into_response(),
    };
    if &req.password != expected {
        return err(StatusCode::UNAUTHORIZED, "wrong password");
    }
    let token = fsutil::random_token();
    if let Ok(mut m) = st.sessions.lock() {
        m.insert(token.clone());
    }
    (
        [
            (
                header::SET_COOKIE,
                format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax"),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        Json(json!({"ok": true})),
    )
        .into_response()
}

async fn logout(State(st): State<SharedState>, headers: HeaderMap) -> Response {
    if let Some(tok) = session_token(&headers) {
        if let Ok(mut m) = st.sessions.lock() {
            m.remove(&tok);
        }
    }
    (
        [(
            header::SET_COOKIE,
            format!("{COOKIE}=; Path=/; HttpOnly; Max-Age=0"),
        )],
        Json(json!({"ok": true})),
    )
        .into_response()
}

async fn list_vhosts(State(st): State<SharedState>) -> Response {
    let paths = st.paths.clone();
    let rows = tokio::task::spawn_blocking(move || vhost::summarize(&paths))
        .await
        .unwrap_or_default();
    let summaries: Vec<&vhost::VhostSummary> = rows.iter().map(|(_, s)| s).collect();
    Json(summaries).into_response()
}

async fn get_raw(State(st): State<SharedState>, Path(id): Path<String>) -> Response {
    let paths = st.paths.clone();
    let key = id.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let vf = vhost::find(&paths, &key).map_err(|e| e.to_string())?;
        vhost::read_raw(&vf).map_err(|e| e.to_string())
    })
    .await;
    match result {
        Ok(Ok(content)) => Json(json!({"id": id, "content": content})).into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_REQUEST, e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Validate one site-block file; returns Err(message) on failure.
fn validate_block(paths: &Paths, path: &std::path::Path) -> Result<(), String> {
    if !caddy::caddy_available() {
        return Ok(());
    }
    caddy::validate_site(paths, path)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

async fn put_raw(
    State(st): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<SaveReq>,
) -> Response {
    let paths = st.paths.clone();
    let content = req.content;
    let outcome = tokio::task::spawn_blocking(move || -> Result<Option<String>, String> {
        let vf = vhost::find(&paths, &id).map_err(|e| e.to_string())?;
        let doc = crate::caddyfile::parser::Document::parse(&content);
        if doc.sites().len() != 1 {
            return Err("content must contain exactly one site block (address + { ... })".into());
        }
        let old = vhost::read_raw(&vf).map_err(|e| e.to_string())?;
        fsutil::atomic_write(&vf.path, &content).map_err(|e| e.to_string())?;
        if let Err(e) = validate_block(&paths, &vf.path) {
            let _ = fsutil::atomic_write(&vf.path, &old);
            return Err(format!("validation failed, reverted:\n{e}"));
        }
        Ok(Some(old))
    })
    .await;

    match outcome {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return err(StatusCode::BAD_REQUEST, e),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }

    let mut reloaded = false;
    let mut reload_error = None;
    if req.reload {
        let paths = st.paths.clone();
        let r = tokio::task::spawn_blocking(move || caddy::reload(&paths))
            .await
            .map(|r| r.map_err(|e| e.to_string()));
        match r {
            Ok(Ok(_)) => reloaded = true,
            Ok(Err(e)) => reload_error = Some(e),
            Err(e) => reload_error = Some(e.to_string()),
        }
    }
    Json(json!({"ok": true, "reloaded": reloaded, "reload_error": reload_error})).into_response()
}

async fn toggle_vhost(
    State(st): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<ToggleReq>,
) -> Response {
    let paths = st.paths.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(bool,), String> {
        let all = vhost::scan(&paths);
        let vf = all
            .iter()
            .find(|v| v.id == id)
            .cloned()
            .ok_or_else(|| format!("no vhost named `{id}`"))?;
        let turning_on = vf.status == vhost::Status::Off;
        if turning_on {
            validate_block(&paths, &vf.path).map_err(|e| format!("validation failed:\n{e}"))?;
        }
        vhost::set_status(&vf, &paths, turning_on).map_err(|e| e.to_string())?;
        Ok((turning_on,))
    })
    .await;

    let turned_on = match result {
        Ok(Ok(v)) => v.0,
        Ok(Err(e)) => return err(StatusCode::BAD_REQUEST, e),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    let mut reloaded = false;
    let mut reload_error = None;
    if req.reload {
        let paths = st.paths.clone();
        match tokio::task::spawn_blocking(move || caddy::reload(&paths)).await {
            Ok(Ok(_)) => reloaded = true,
            Ok(Err(e)) => reload_error = Some(e.to_string()),
            Err(e) => reload_error = Some(e.to_string()),
        }
    }
    Json(json!({
        "ok": true,
        "status": if turned_on { "on" } else { "off" },
        "reloaded": reloaded,
        "reload_error": reload_error,
    }))
    .into_response()
}

async fn delete_vhost(
    State(st): State<SharedState>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let paths = st.paths.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let all = vhost::scan(&paths);
        let vf = all
            .iter()
            .find(|v| v.id == id)
            .cloned()
            .ok_or_else(|| format!("no vhost named `{id}`"))?;
        vhost::soft_delete(&vf, &paths)
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return err(StatusCode::BAD_REQUEST, e),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }

    let mut reloaded = false;
    let mut reload_error = None;
    if q.get("reload").map(|v| v == "true").unwrap_or(false) {
        let paths = st.paths.clone();
        match tokio::task::spawn_blocking(move || caddy::reload(&paths)).await {
            Ok(Ok(_)) => reloaded = true,
            Ok(Err(e)) => reload_error = Some(e.to_string()),
            Err(e) => reload_error = Some(e.to_string()),
        }
    }
    Json(json!({"ok": true, "reloaded": reloaded, "reload_error": reload_error})).into_response()
}

async fn create_vhost(State(st): State<SharedState>, Json(req): Json<CreateReq>) -> Response {
    let paths = st.paths.clone();
    let result = if req.source.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        let source = req.source.unwrap_or_default();
        tokio::task::spawn_blocking(move || {
            vhost::create_vhost_source(&paths, &source).map_err(|e| e.to_string())
        })
        .await
    } else {
        let domains: Vec<String> = req
            .domains
            .unwrap_or_default()
            .split([',', ' ', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        if domains.is_empty() {
            return err(StatusCode::BAD_REQUEST, "at least one domain is required");
        }
        let upstream = req.upstream.trim().to_string();
        let tls = req.tls.clone();
        let watch_log = req.watch_log;
        tokio::task::spawn_blocking(move || {
            vhost::create_vhost_file(&paths, &domains, &upstream, &tls, watch_log)
                .map_err(|e| e.to_string())
        })
        .await
    };

    let target = match result {
        Ok(Ok(target)) => target,
        Ok(Err(e)) => return err(StatusCode::BAD_REQUEST, e),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    let mut reloaded = false;
    let mut reload_error = None;
    let paths = st.paths.clone();
    match tokio::task::spawn_blocking(move || caddy::reload(&paths)).await {
        Ok(Ok(_)) => reloaded = true,
        Ok(Err(e)) => reload_error = Some(e.to_string()),
        Err(e) => reload_error = Some(e.to_string()),
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "ok": true,
            "id": target.file_stem().unwrap_or_default().to_string_lossy(),
            "file": target.display().to_string(),
            "reloaded": reloaded,
            "reload_error": reload_error,
        })),
    )
        .into_response()
}

async fn reload_now(State(st): State<SharedState>) -> Response {
    let paths = st.paths.clone();
    let r = tokio::task::spawn_blocking(move || caddy::reload(&paths)).await;
    match r {
        Ok(Ok(output)) => Json(json!({"ok": true, "output": output})).into_response(),
        Ok(Err(e)) => err(StatusCode::BAD_GATEWAY, e.to_string()),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/* ---------- self-update ---------- */

#[derive(serde::Serialize)]
struct UpdateCheck {
    current: &'static str,
    latest: Option<String>,
    up_to_date: bool,
    supported: bool,
    error: Option<String>,
}

async fn update_check() -> Response {
    let supported = crate::selfupdate::asset_name().is_some();
    if !supported {
        return Json(UpdateCheck {
            current: env!("CARGO_PKG_VERSION"),
            latest: None,
            up_to_date: true,
            supported: false,
            error: Some("auto-update unsupported on this platform".into()),
        })
        .into_response();
    }
    let result = tokio::task::spawn_blocking(crate::selfupdate::latest_version).await;
    match result.unwrap_or_else(|e| Err(anyhow::anyhow!(e.to_string()))) {
        Ok(latest) => Json(UpdateCheck {
            current: env!("CARGO_PKG_VERSION"),
            up_to_date: !crate::selfupdate::is_newer(&latest, env!("CARGO_PKG_VERSION")),
            latest: Some(latest),
            supported: true,
            error: None,
        })
        .into_response(),
        Err(e) => Json(UpdateCheck {
            current: env!("CARGO_PKG_VERSION"),
            latest: None,
            up_to_date: false,
            supported: true,
            error: Some(e.to_string()),
        })
        .into_response(),
    }
}

async fn update_apply() -> Response {
    let latest = match tokio::task::spawn_blocking(crate::selfupdate::latest_version)
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!(e.to_string())))
    {
        Ok(v) => v,
        Err(e) => {
            return err(
                StatusCode::BAD_GATEWAY,
                format!("version check failed: {e}"),
            )
        }
    };
    if !crate::selfupdate::is_newer(&latest, env!("CARGO_PKG_VERSION")) {
        return Json(json!({
            "ok": true,
            "updated": false,
            "message": format!("already on v{}", env!("CARGO_PKG_VERSION"))
        }))
        .into_response();
    }

    // download + verify + install (bounded work, safe inside this request)
    let install = tokio::task::spawn_blocking(move || crate::selfupdate::install_version(&latest))
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!(e.to_string())));
    let message = match install {
        Ok(m) => m,
        Err(e) => return err(StatusCode::BAD_GATEWAY, e.to_string()),
    };

    // hand the restart to a transient unit so it survives our own shutdown
    let restart_scheduled = tokio::task::spawn_blocking(crate::selfupdate::schedule_restart)
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!(e.to_string())))
        .is_ok();

    Json(json!({
        "ok": true,
        "updated": true,
        "restarting": restart_scheduled,
        "message": message,
    }))
    .into_response()
}

/* ---------- wiring ---------- */

pub async fn run(host: &str, port: u16, paths: Paths) -> anyhow::Result<()> {
    let password = std::env::var("CADDEDIT_PASSWORD")
        .ok()
        .filter(|p| !p.is_empty());

    let state: SharedState = Arc::new(AppState {
        paths,
        password,
        sessions: Mutex::new(HashSet::new()),
    });

    let api = Router::new()
        .route("/api/status", get(status))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/vhosts", get(list_vhosts).post(create_vhost))
        .route("/api/vhosts/{id}/raw", get(get_raw).put(put_raw))
        .route("/api/vhosts/{id}/toggle", post(toggle_vhost))
        .route("/api/vhosts/{id}", axum::routing::delete(delete_vhost))
        .route("/api/reload", post(reload_now))
        .route("/api/update/check", get(update_check))
        .route("/api/update", post(update_apply))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth_mw))
        .with_state(state);

    let app = api.merge(Router::new().fallback(static_handler));

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!(
        "{} listening on http://{addr}",
        "caddedit".bright_cyan().bold()
    );
    axum::serve(listener, app).await?;
    Ok(())
}

use owo_colors::OwoColorize;
