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

pub mod auth;

const COOKIE: &str = "caddedit_session";

#[derive(RustEmbed)]
#[folder = "web/"]
struct Assets;

pub struct AppState {
    paths: Paths,
    password: Option<String>,
    sessions: Mutex<HashSet<String>>,
    update: Mutex<UpdateState>,
    refresh_token: Mutex<Option<auth::RefreshToken>>,
    access_tokens: Mutex<HashMap<String, u64>>, // token -> expiry ms
}

#[derive(Clone, Default, serde::Serialize)]
struct UpdateState {
    running: bool,
    /// idle | checking | downloading | installing | restarting | done | error
    stage: String,
    message: String,
    target: Option<String>,
    /// unix millis when the update started (0 = not started)
    started_at: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?;
    let s = v.to_str().ok()?;
    let tok = s.strip_prefix("Bearer ")?;
    Some(tok.trim().to_string())
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

fn is_access_token_valid(st: &AppState, token: &str) -> bool {
    let mut map = match st.access_tokens.lock() {
        Ok(m) => m,
        Err(_) => return false,
    };
    if let Some(&expiry) = map.get(token) {
        if expiry > now_ms() {
            return true;
        } else {
            // expired, remove lazily
            map.remove(token);
        }
    }
    false
}

fn is_cli_user_agent(ua: Option<&String>) -> bool {
    match ua {
        Some(s) => s.starts_with("caddedit-cli/"),
        None => false,
    }
}

fn authenticated(st: &AppState, headers: &HeaderMap) -> bool {
    if st.password.is_none() {
        return true;
    }
    // Bearer access token (CLI) takes precedence — must present valid CLI User-Agent
    if let Some(tok) = bearer_token(headers) {
        let ua = user_agent(headers);
        if !is_cli_user_agent(ua.as_ref()) {
            return false;
        }
        if is_access_token_valid(st, &tok) {
            return true;
        }
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
    // open endpoints: status and auth exchange (login + refresh)
    let open = path == "/api/status" || path == "/api/login" || path == "/api/auth/refresh";
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
    #[serde(default)]
    source: Option<String>,
}
fn default_tls() -> String {
    "internal".into()
}

#[derive(Deserialize)]
struct RefreshReq {
    refresh_token: String,
}

/* ---------- handlers ---------- */

async fn status(State(st): State<SharedState>, headers: HeaderMap) -> Response {
    let has_refresh = st
        .refresh_token
        .lock()
        .map(|m| m.is_some())
        .unwrap_or(false);
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "config_path": st.paths.caddyfile.display().to_string(),
        "vhosts_dir": st.paths.vhosts_dir.display().to_string(),
        "caddy_available": caddy::caddy_available(),
        "auth_required": st.password.is_some(),
        "authenticated": authenticated(&st, &headers),
        "has_refresh_token": has_refresh,
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

/* ---------- auth token handlers ---------- */

async fn token_status(State(st): State<SharedState>) -> Response {
    let rt = st
        .refresh_token
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let access_count = st
        .access_tokens
        .lock()
        .map(|m| {
            let now = now_ms();
            m.values().filter(|&&exp| exp > now).count()
        })
        .unwrap_or(0);
    Json(json!({
        "has_refresh_token": rt.is_some(),
        "refresh_created_at": rt.as_ref().map(|r| r.created_at),
        "refresh_created_by_ua": rt.as_ref().and_then(|r| r.created_by_ua.clone()),
        "active_access_tokens": access_count,
        "access_ttl_ms": auth::ACCESS_TOKEN_TTL_MS,
    }))
    .into_response()
}

async fn generate_refresh_token(State(st): State<SharedState>, headers: HeaderMap) -> Response {
    // requires already authenticated (via session or access token)
    let ua = user_agent(&headers);
    let new_token = fsutil::random_token();
    let rt = auth::RefreshToken {
        token: new_token.clone(),
        created_at: now_ms(),
        created_by_ua: ua.clone(),
    };
    {
        let mut guard = match st.refresh_token.lock() {
            Ok(g) => g,
            Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "lock poisoned"),
        };
        *guard = Some(rt.clone());
        // persist
        if let Err(e) = auth::save_refresh(&st.paths, &guard) {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to persist refresh token: {e}"),
            );
        }
    }
    // invalidate all existing access tokens (old refresh no longer usable indirectly)
    if let Ok(mut m) = st.access_tokens.lock() {
        m.clear();
    }
    eprintln!(
        "refresh token rotated (ua={}) at {}",
        ua.unwrap_or_else(|| "-".into()),
        rt.created_at
    );
    Json(json!({
        "ok": true,
        "refresh_token": new_token,
        "created_at": rt.created_at,
        "note": "copy now — this token is shown once. Old refresh token is invalidated. Use it to generate day-long access tokens via POST /api/auth/refresh."
    }))
    .into_response()
}

async fn refresh_access_token(
    State(st): State<SharedState>,
    headers: HeaderMap,
    Json(req): Json<RefreshReq>,
) -> Response {
    let ua = user_agent(&headers);
    // CLI must identify itself via User-Agent
    if !is_cli_user_agent(ua.as_ref()) {
        return err(
            StatusCode::BAD_REQUEST,
            "missing or invalid User-Agent: expected caddedit-cli/<version>",
        );
    }
    let stored = st
        .refresh_token
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let rt = match stored {
        Some(r) => r,
        None => {
            return err(
                StatusCode::NOT_FOUND,
                "no refresh token configured — generate one in Settings",
            )
        }
    };
    if req.refresh_token.trim() != rt.token {
        return err(StatusCode::UNAUTHORIZED, "invalid refresh token");
    }
    let access = fsutil::random_token();
    let expires_at = now_ms() + auth::ACCESS_TOKEN_TTL_MS;
    if let Ok(mut m) = st.access_tokens.lock() {
        // purge expired
        let now = now_ms();
        m.retain(|_, &mut exp| exp > now);
        m.insert(access.clone(), expires_at);
    }
    eprintln!(
        "access token issued (ua={}) expires_at={}",
        ua.unwrap_or_else(|| "-".into()),
        expires_at
    );
    Json(json!({
        "ok": true,
        "access_token": access,
        "token_type": "Bearer",
        "expires_at": expires_at,
        "expires_in": auth::ACCESS_TOKEN_TTL_MS / 1000,
    }))
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
    /// markdown release notes (truncated)
    notes: Option<String>,
    published_at: Option<String>,
    channel: String,
}

#[derive(Deserialize)]
struct UpdateQuery {
    channel: Option<String>,
}

async fn update_check(Query(q): Query<UpdateQuery>) -> Response {
    let channel = q.channel.as_deref().unwrap_or("stable");
    let supported = crate::selfupdate::asset_name().is_some();
    if !supported {
        return Json(UpdateCheck {
            current: env!("CARGO_PKG_VERSION"),
            latest: None,
            up_to_date: true,
            supported: false,
            error: Some("auto-update unsupported on this platform".into()),
            notes: None,
            published_at: None,
            channel: channel.to_string(),
        })
        .into_response();
    }
    let ch = channel.to_string();
    let current = env!("CARGO_PKG_VERSION").to_string();
    let result = tokio::task::spawn_blocking(move || {
        if ch == "nightly" {
            // nightly should also consider stable — if stable is newer than nightly, show stable
            let stable_info = crate::selfupdate::release_info_for("stable").ok();
            let nightly_info = crate::selfupdate::release_info_for("nightly").ok();
            let stable_newer = stable_info
                .as_ref()
                .map(|i| crate::selfupdate::is_newer(&i.version, &current))
                .unwrap_or(false);
            let nightly_newer = nightly_info
                .as_ref()
                .map(|i| crate::selfupdate::is_newer_for(&i.version, &current, "nightly"))
                .unwrap_or(false);
            // pick the newest available: nightly if it is newer than both current and stable
            let pick_nightly = nightly_newer
                && stable_info
                    .as_ref()
                    .map(|s| {
                        crate::selfupdate::is_newer_for(
                            nightly_info.as_ref().unwrap().version.as_str(),
                            &s.version,
                            "nightly",
                        )
                    })
                    .unwrap_or(true);
            let info = if pick_nightly {
                nightly_info.unwrap()
            } else if stable_newer {
                stable_info.unwrap()
            } else {
                // up to date — return nightly info if exists, else stable
                nightly_info
                    .or(stable_info)
                    .ok_or_else(|| anyhow::anyhow!("no release found"))?
            };
            let mut notes = info.notes.clone();
            let mut up_to_date = if pick_nightly {
                !crate::selfupdate::is_newer_for(&info.version, &current, "nightly")
            } else {
                !stable_newer
            };
            // try compare for better notes
            let base = format!("v{current}");
            let head = if pick_nightly {
                "nightly".to_string()
            } else {
                format!("v{}", info.version)
            };
            if let Ok(Some(cmp)) = crate::selfupdate::compare_notes(&base, &head) {
                if cmp.contains("Already ahead") {
                    up_to_date = true;
                    let is_generic = notes
                        .as_ref()
                        .map(|n| {
                            let t = n.trim();
                            t.starts_with("**Full Changelog**") && t.lines().count() <= 4
                        })
                        .unwrap_or(true);
                    if is_generic {
                        notes = Some(cmp);
                    }
                } else {
                    let is_generic = notes
                        .as_ref()
                        .map(|n| {
                            let t = n.trim();
                            t.starts_with("**Full Changelog**") && t.lines().count() <= 3
                        })
                        .unwrap_or(true);
                    if is_generic || notes.is_none() {
                        notes = Some(cmp);
                    }
                    up_to_date = false;
                }
            }
            Ok::<_, anyhow::Error>((info, notes, up_to_date))
        } else {
            let info = crate::selfupdate::release_info_for(&ch)?;
            let base = format!("v{current}");
            let head = format!("v{}", info.version);
            let mut notes = info.notes.clone();
            let mut up_to_date = !crate::selfupdate::is_newer_for(&info.version, &current, &ch);
            if let Ok(Some(cmp)) = crate::selfupdate::compare_notes(&base, &head) {
                if cmp.contains("Already ahead") {
                    up_to_date = true;
                    let is_generic = notes
                        .as_ref()
                        .map(|n| {
                            let t = n.trim();
                            t.starts_with("**Full Changelog**") && t.lines().count() <= 4
                        })
                        .unwrap_or(true);
                    if is_generic {
                        notes = Some(cmp);
                    }
                } else {
                    let is_generic = notes
                        .as_ref()
                        .map(|n| {
                            let t = n.trim();
                            t.starts_with("**Full Changelog**") && t.lines().count() <= 3
                        })
                        .unwrap_or(true);
                    if is_generic || notes.is_none() {
                        notes = Some(cmp);
                    }
                    up_to_date = false;
                }
            }
            Ok::<_, anyhow::Error>((info, notes, up_to_date))
        }
    })
    .await;
    match result.unwrap_or_else(|e| Err(anyhow::anyhow!(e.to_string()))) {
        Ok((info, notes, up_to_date)) => Json(UpdateCheck {
            current: env!("CARGO_PKG_VERSION"),
            up_to_date,
            latest: Some(info.version.clone()),
            supported: true,
            error: None,
            notes,
            published_at: info.published_at,
            channel: channel.to_string(),
        })
        .into_response(),
        Err(e) => Json(UpdateCheck {
            current: env!("CARGO_PKG_VERSION"),
            latest: None,
            up_to_date: false,
            supported: true,
            error: Some(e.to_string()),
            notes: None,
            published_at: None,
            channel: channel.to_string(),
        })
        .into_response(),
    }
}

async fn update_status(State(st): State<SharedState>) -> Response {
    let u = st.update.lock().unwrap_or_else(|p| p.into_inner()).clone();
    Json(u).into_response()
}

async fn update_apply(State(st): State<SharedState>, Query(q): Query<UpdateQuery>) -> Response {
    let channel = q.channel.unwrap_or_else(|| "stable".to_string());
    {
        let mut u = st.update.lock().unwrap_or_else(|p| p.into_inner());
        if u.running {
            return err(StatusCode::CONFLICT, "update already in progress");
        }
        u.running = true;
        u.stage = "checking".into();
        u.message = format!("checking GitHub releases ({channel})");
        u.target = None;
        u.started_at = now_ms();
    }

    let worker = st.clone();
    tokio::spawn(async move {
        let set = |stage: &str, msg: &str| {
            if let Ok(mut u) = worker.update.lock() {
                u.stage = stage.into();
                u.message = msg.into();
            }
        };
        let result: anyhow::Result<String> = async {
            let ch = channel.clone();
            let latest =
                tokio::task::spawn_blocking(move || crate::selfupdate::latest_version_for(&ch))
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))??;
            if !crate::selfupdate::is_newer_for(&latest, env!("CARGO_PKG_VERSION"), &channel) {
                return Ok(format!(
                    "already on v{} (channel {channel})",
                    env!("CARGO_PKG_VERSION")
                ));
            }
            if let Ok(mut u) = worker.update.lock() {
                u.target = Some(latest.clone());
            }
            let display = if channel == "nightly" {
                format!("downloading nightly {latest}")
            } else {
                format!("downloading v{latest}")
            };
            set("downloading", &display);
            let install =
                tokio::task::spawn_blocking(move || crate::selfupdate::install_version(&latest))
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))??;
            set("restarting", "handing over to systemd");
            tokio::task::spawn_blocking(crate::selfupdate::schedule_restart)
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))??;
            Ok(install)
        }
        .await;

        let mut u = worker.update.lock().unwrap_or_else(|p| p.into_inner());
        u.running = false;
        match result {
            Ok(msg) => {
                u.stage = "done".into();
                u.message = msg;
            }
            Err(e) => {
                u.stage = "error".into();
                u.message = e.to_string();
            }
        }
    });

    Json(json!({"ok": true, "started": true})).into_response()
}

/* ---------- wiring ---------- */

pub async fn run(host: &str, port: u16, paths: Paths) -> anyhow::Result<()> {
    let password = std::env::var("CADDEDIT_PASSWORD")
        .ok()
        .filter(|p| !p.is_empty());

    let initial_refresh = auth::load_refresh(&paths);

    let state: SharedState = Arc::new(AppState {
        paths,
        password,
        sessions: Mutex::new(HashSet::new()),
        update: Mutex::new(UpdateState::default()),
        refresh_token: Mutex::new(initial_refresh),
        access_tokens: Mutex::new(HashMap::new()),
    });

    let api = Router::new()
        .route("/api/status", get(status))
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/auth/tokens/status", get(token_status))
        .route("/api/auth/tokens/generate", post(generate_refresh_token))
        .route("/api/auth/refresh", post(refresh_access_token))
        .route("/api/vhosts", get(list_vhosts).post(create_vhost))
        .route("/api/vhosts/{id}/raw", get(get_raw).put(put_raw))
        .route("/api/vhosts/{id}/toggle", post(toggle_vhost))
        .route("/api/vhosts/{id}", axum::routing::delete(delete_vhost))
        .route("/api/reload", post(reload_now))
        .route("/api/update/check", get(update_check))
        .route("/api/update/status", get(update_status))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use axum::http::{header, HeaderMap, HeaderValue};
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    fn test_state_with_password(pw: Option<&str>) -> SharedState {
        let tmp = TempDir::new().unwrap();
        let caddyfile = tmp.path().join("Caddyfile");
        std::fs::write(&caddyfile, "{ admin off }\n").unwrap();
        // leak tempdir so paths stay valid for test (we don't need cleanup)
        let _leaked = Box::leak(Box::new(tmp));
        let paths = Paths::resolve(Some(caddyfile), None);
        Arc::new(AppState {
            paths,
            password: pw.map(|s| s.to_string()),
            sessions: Mutex::new(HashSet::new()),
            update: Mutex::new(UpdateState::default()),
            refresh_token: Mutex::new(None),
            access_tokens: Mutex::new(HashMap::new()),
        })
    }

    #[test]
    fn bearer_token_extraction() {
        let mut hm = HeaderMap::new();
        hm.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc123"),
        );
        assert_eq!(bearer_token(&hm).as_deref(), Some("abc123"));
        let mut hm2 = HeaderMap::new();
        hm2.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert!(bearer_token(&hm2).is_none());
    }

    #[test]
    fn user_agent_extraction() {
        let mut hm = HeaderMap::new();
        hm.insert(
            header::USER_AGENT,
            HeaderValue::from_static("caddedit-cli/0.5.2"),
        );
        assert_eq!(user_agent(&hm).as_deref(), Some("caddedit-cli/0.5.2"));
        let hm2 = HeaderMap::new();
        assert!(user_agent(&hm2).is_none());
    }

    #[test]
    fn access_token_valid_for_24h_and_expires() {
        let st = test_state_with_password(Some("secret"));
        let token = "test_access_token";
        let future = now_ms() + 100_000;
        let past = now_ms() - 1000;
        {
            let mut m = st.access_tokens.lock().unwrap();
            m.insert(token.to_string(), future);
        }
        assert!(is_access_token_valid(&st, token));
        {
            let mut m = st.access_tokens.lock().unwrap();
            m.insert(token.to_string(), past);
        }
        assert!(!is_access_token_valid(&st, token));
        // expired should be purged
        assert!(!st.access_tokens.lock().unwrap().contains_key(token));
    }

    #[test]
    fn authenticated_accepts_bearer_and_rejects_old_refresh() {
        let st = test_state_with_password(Some("secret"));
        // no token -> not authenticated
        let hm = HeaderMap::new();
        assert!(!authenticated(&st, &hm));
        // insert valid access token
        let at = "valid_access_123";
        {
            let mut m = st.access_tokens.lock().unwrap();
            m.insert(at.to_string(), now_ms() + 86400000);
        }
        let mut hm2 = HeaderMap::new();
        hm2.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {at}")).unwrap(),
        );
        hm2.insert(
            header::USER_AGENT,
            HeaderValue::from_static("caddedit-cli/0.5.2"),
        );
        // Bearer should authenticate even without session cookie — requires CLI UA
        assert!(authenticated(&st, &hm2));
        // without UA, bearer is rejected
        let mut hm_no_ua = HeaderMap::new();
        hm_no_ua.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {at}")).unwrap(),
        );
        assert!(!authenticated(&st, &hm_no_ua));
        // wrong UA also rejected
        let mut hm_bad_ua = HeaderMap::new();
        hm_bad_ua.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {at}")).unwrap(),
        );
        hm_bad_ua.insert(header::USER_AGENT, HeaderValue::from_static("Mozilla/5.0"));
        assert!(!authenticated(&st, &hm_bad_ua));

        // refresh token itself should NOT authenticate (only access token)
        let rt = auth::RefreshToken {
            token: "refresh_old".into(),
            created_at: now_ms(),
            created_by_ua: Some("caddedit-cli/0.5.2".into()),
        };
        *st.refresh_token.lock().unwrap() = Some(rt);
        let mut hm3 = HeaderMap::new();
        hm3.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer refresh_old"),
        );
        hm3.insert(
            header::USER_AGENT,
            HeaderValue::from_static("caddedit-cli/0.5.2"),
        );
        assert!(!authenticated(&st, &hm3));
    }

    #[test]
    fn ttl_is_one_day() {
        assert_eq!(auth::ACCESS_TOKEN_TTL_MS, 86400000);
    }

    #[tokio::test]
    async fn refresh_flow_rotation_invalidates_old() {
        let st = test_state_with_password(Some("secret"));
        // simulate first refresh token generation (like handler does)
        let rt1 = "rt1_one_time";
        *st.refresh_token.lock().unwrap() = Some(auth::RefreshToken {
            token: rt1.into(),
            created_at: now_ms(),
            created_by_ua: Some("caddedit-cli/0.5.2".into()),
        });
        // generate access token via rt1
        {
            let mut m = st.access_tokens.lock().unwrap();
            m.insert("at1".into(), now_ms() + auth::ACCESS_TOKEN_TTL_MS);
        }
        assert_eq!(st.access_tokens.lock().unwrap().len(), 1);
        // rotate refresh token (handler clears access_tokens)
        *st.refresh_token.lock().unwrap() = Some(auth::RefreshToken {
            token: "rt2_new".into(),
            created_at: now_ms(),
            created_by_ua: Some("Mozilla/5.0".into()),
        });
        st.access_tokens.lock().unwrap().clear();
        assert!(st.access_tokens.lock().unwrap().is_empty());
        // old rt1 no longer matches
        let stored = st.refresh_token.lock().unwrap().clone().unwrap();
        assert_ne!(stored.token, rt1);
        assert_eq!(stored.token, "rt2_new");
    }
}
