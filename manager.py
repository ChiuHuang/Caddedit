import html
import json
import hashlib
import hmac
import os
import re
import shutil
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import httpx
from dotenv import load_dotenv
from fastapi import Cookie, Depends, FastAPI, Header, HTTPException, Response
from fastapi.responses import HTMLResponse
from pydantic import BaseModel, Field



# Load environment variables from .env file
load_dotenv()

ADMIN_PASSWORD = os.getenv("CADDEDIT_PASSWORD")
if not ADMIN_PASSWORD:
    raise ValueError("CADDEDIT_PASSWORD environment variable must be set.")

AUTH_COOKIE = "caddedit_auth"
CADDYFILE_PATH = Path(os.getenv("CADDYFILE_PATH", "/etc/caddy/Caddyfile"))
VHOSTS_DIR = Path(os.getenv("VHOSTS_DIR", "/etc/caddy/vhosts"))
ENABLED_DIR = VHOSTS_DIR / "enabled"
DISABLED_DIR = VHOSTS_DIR / "disabled"
BACKUP_DIR = Path(os.getenv("CADDY_BACKUP_DIR", "/etc/caddy/caddedit-backups"))
COHERE_API_KEY = os.getenv("COHERE_API_KEY")
COHERE_MODEL = os.getenv("COHERE_MODEL", "command-a-03-2025")
RELOAD_COMMAND = os.getenv("CADDY_RELOAD_COMMAND", "caddy reload --config /etc/caddy/Caddyfile")
LEARNED_RULES_PATH = Path(os.getenv("HARDCODED_RULES_PATH", "/projects/Caddedit/hardcoded-rules.json"))
DISABLE_AI = os.getenv("DISABLE_AI", "false").lower() in ("true", "1", "yes")

# Branding is configurable per-deployment - not every install of this is
# running on the operator's own domain, so neither the on-screen title nor
# the placeholder/example domain should be hardcoded to one person's setup.
SITE_TITLE = os.getenv("SITE_TITLE", "TXG1 ROUTER").strip() or "TXG1 ROUTER"
SITE_DOMAIN_HINT = os.getenv("SITE_DOMAIN_HINT", "").strip().lstrip(".")

# Load HTML templates
TEMPLATES_DIR = Path(__file__).parent / "templates"


def render_template(text: str) -> str:
    return (
        text.replace("{{SITE_TITLE}}", html.escape(SITE_TITLE))
        .replace("{{SITE_DOMAIN_HINT}}", html.escape(SITE_DOMAIN_HINT))
    )


LOGIN_HTML = render_template((TEMPLATES_DIR / "login.html").read_text(encoding="utf-8"))
HTML = render_template((TEMPLATES_DIR / "index.html").read_text(encoding="utf-8"))

app = FastAPI(title=SITE_TITLE)




class RoutePayload(BaseModel):
    original_id: str | None = None
    status: str = "ON"
    source: str = Field(min_length=1)


class ConfigPayload(BaseModel):
    source: str = Field(min_length=1)


class AiParsePayload(BaseModel):
    source: str = Field(min_length=1)


class LoginPayload(BaseModel):
    password: str


class HardcodedRulePayload(BaseModel):
    source: str = Field(min_length=1)
    parsed: dict[str, Any]


def auth_token() -> str:
    """The single permanent credential behind both the browser cookie and
    the CLI's bearer token. It's deterministic from the unlock password (and
    optional CADDEDIT_AUTH_SECRET), so it never needs separate storage on the
    server and never silently expires - it only changes if the password (or
    secret) changes."""
    secret = os.getenv("CADDEDIT_AUTH_SECRET", ADMIN_PASSWORD)
    return hmac.new(secret.encode("utf-8"), b"caddedit-auth-v1", hashlib.sha256).hexdigest()


def _token_matches(candidate: str | None) -> bool:
    return bool(candidate) and hmac.compare_digest(candidate, auth_token())


def require_auth(
    caddedit_auth: str | None = Cookie(default=None),
    authorization: str | None = Header(default=None),
) -> None:
    """Accepts either the browser's HttpOnly session cookie, or a CLI/script
    bearer token (`Authorization: Bearer <token>`). Both carry the same
    underlying value - see auth_token()."""
    if _token_matches(caddedit_auth):
        return
    if authorization and authorization.lower().startswith("bearer "):
        if _token_matches(authorization[7:].strip()):
            return
    raise HTTPException(status_code=401, detail="Login required")


def read_caddyfile() -> str:
    if not CADDYFILE_PATH.exists():
        return "{\n    email admin@example.com\n}\n\nexample.com {\n    reverse_proxy 127.0.0.1:8080\n}\n"
    return CADDYFILE_PATH.read_text(encoding="utf-8")


def backup_caddyfile() -> None:
    if not CADDYFILE_PATH.exists():
        return
    BACKUP_DIR.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    shutil.copy2(CADDYFILE_PATH, BACKUP_DIR / f"Caddyfile.{stamp}.bak")


def write_caddyfile(content: str) -> None:
    backup_caddyfile()
    CADDYFILE_PATH.parent.mkdir(parents=True, exist_ok=True)
    CADDYFILE_PATH.write_text(content.rstrip() + "\n", encoding="utf-8")


def run_caddy_reload() -> dict[str, Any]:
    command = RELOAD_COMMAND.split()
    result = subprocess.run(command, capture_output=True, text=True, timeout=20, check=False)
    if result.returncode != 0:
        raise HTTPException(status_code=500, detail=(result.stderr or result.stdout or "Reload failed").strip())
    return {"ok": True, "output": (result.stdout or "Caddy reloaded").strip()}


def route_filename(domain: str) -> str:
    clean = domain.split("{", 1)[0].strip()
    if SITE_DOMAIN_HINT and clean.lower().endswith("." + SITE_DOMAIN_HINT.lower()):
        clean = clean[: -(len(SITE_DOMAIN_HINT) + 1)]
    clean = clean.replace(":", "_").replace("*", "wildcard").replace("/", "fallback")
    clean = re.sub(r"[^A-Za-z0-9_.-]+", "_", clean).strip("._-")
    if not clean:
        raise HTTPException(status_code=400, detail="Invalid route name")
    return f"{clean}.caddy"


def route_path(domain: str, status: str) -> Path:
    target = ENABLED_DIR if status.upper() == "ON" else DISABLED_DIR
    return target / route_filename(domain)


def ensure_vhost_dirs() -> None:
    ENABLED_DIR.mkdir(parents=True, exist_ok=True)
    DISABLED_DIR.mkdir(parents=True, exist_ok=True)


def read_vhost_routes() -> list[dict[str, Any]]:
    ensure_vhost_dirs()
    routes: list[dict[str, Any]] = []
    for status, directory in (("ON", ENABLED_DIR), ("OFF", DISABLED_DIR)):
        for path in sorted(directory.glob("*.caddy")):
            source = path.read_text(encoding="utf-8").strip()
            if not source:
                continue
            route = parse_site_block(source)
            route["status"] = status
            route["file"] = str(path)
            route["id"] = block_id(str(path))
            routes.append(route)
    return sorted(routes, key=lambda item: item["domain"])


def find_vhost_route(route_id: str) -> dict[str, Any]:
    for route in read_vhost_routes():
        if route["id"] == route_id:
            return route
    raise HTTPException(status_code=404, detail="Route not found")


def strip_comments(line: str) -> str:
    in_quote = False
    escaped = False
    for idx, char in enumerate(line):
        if escaped:
            escaped = False
            continue
        if char == "\\":
            escaped = True
            continue
        if char == '"':
            in_quote = not in_quote
            continue
        if char == "#" and not in_quote:
            return line[:idx]
    return line


def brace_delta(line: str) -> int:
    clean = strip_comments(line)
    clean = re.sub(r'"(?:\\.|[^"\\])*"', '""', clean)
    clean = re.sub(r"\{[A-Za-z0-9_.:-]+\}", "", clean)
    return clean.count("{") - clean.count("}")


def block_id(source: str) -> str:
    return hashlib.sha256(source.encode("utf-8")).hexdigest()[:16]


def source_key(source: str) -> str:
    normalized = "\n".join(line.rstrip() for line in source.strip().splitlines())
    return hashlib.sha256(normalized.encode("utf-8")).hexdigest()


def read_learned_rules() -> dict[str, Any]:
    if not LEARNED_RULES_PATH.exists():
        return {}
    try:
        return json.loads(LEARNED_RULES_PATH.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return {}


def write_learned_rules(rules: dict[str, Any]) -> None:
    LEARNED_RULES_PATH.parent.mkdir(parents=True, exist_ok=True)
    LEARNED_RULES_PATH.write_text(json.dumps(rules, indent=2, ensure_ascii=False), encoding="utf-8")


def block_header(source: str) -> str:
    for line in source.splitlines():
        stripped = line.lstrip("\ufeff").strip()
        if stripped and not stripped.startswith("#"):
            return stripped.removesuffix("{").strip()
    return ""


def split_top_level_blocks(content: str) -> tuple[list[str], list[dict[str, Any]]]:
    prefix: list[str] = []
    blocks: list[dict[str, Any]] = []
    current: list[str] = []
    depth = 0
    in_block = False

    for line in content.splitlines():
        delta = brace_delta(line)
        starts_block = "{" in strip_comments(line) and depth == 0

        if starts_block:
            in_block = True
            current = [line]
            depth += delta
            if depth <= 0:
                source = "\n".join(current)
                blocks.append({"id": block_id(source), "header": block_header(source), "source": source})
                current = []
                in_block = False
                depth = 0
            continue

        if in_block:
            current.append(line)
            depth += delta
            if depth <= 0:
                source = "\n".join(current)
                blocks.append({"id": block_id(source), "header": block_header(source), "source": source})
                current = []
                in_block = False
                depth = 0
        else:
            prefix.append(line)

    if current:
        source = "\n".join(current)
        blocks.append({"id": block_id(source), "header": block_header(source), "source": source})

    return prefix, blocks


def is_global_or_snippet(header: str) -> bool:
    return header == "" or header.startswith("(") or header == "{"


def parse_site_block(source: str) -> dict[str, Any]:
    lines = source.splitlines()
    header = block_header(source)
    inner = lines[1:-1] if len(lines) >= 2 else []
    route = {
        "id": block_id(source),
        "domain": header,
        "source": source,
        "tls": {"mode": "default", "raw": ""},
        "request_watch_log": True,
        "reverse_proxy": "",
        "directives": [],
        "complex": False,
        "parser": "hardcoded",
    }

    learned = read_learned_rules().get(source_key(source))
    if learned:
        route.update({key: value for key, value in learned.items() if key not in {"id", "source", "status", "file"}})
        route["source"] = source
        route["parser"] = "learned"
        return route

    i = 0
    while i < len(inner):
        raw_line = inner[i]
        stripped = raw_line.strip()
        if not stripped or stripped.startswith("#"):
            route["directives"].append({"type": "raw", "raw": raw_line})
            i += 1
            continue

        keyword = stripped.split(None, 1)[0]
        rest = stripped[len(keyword):].strip()

        if keyword == "import" and rest == "request_watch_log":
            route["request_watch_log"] = True
            i += 1
            continue

        if keyword == "tls":
            if brace_delta(raw_line) > 0:
                collected = [raw_line]
                depth = brace_delta(raw_line)
                i += 1
                while i < len(inner) and depth > 0:
                    collected.append(inner[i])
                    depth += brace_delta(inner[i])
                    i += 1
                raw = "\n".join(collected)
                mode = "cloudflare" if "cloudflare" in raw.lower() else "custom"
                route["tls"] = {"mode": mode, "raw": raw}
                if mode == "custom":
                    route["complex"] = True
                continue
            route["tls"] = {"mode": rest or "internal", "raw": raw_line}
            i += 1
            continue

        if keyword == "reverse_proxy" and brace_delta(raw_line) <= 0:
            route["reverse_proxy"] = rest
            i += 1
            continue

        if brace_delta(raw_line) > 0:
            collected = [raw_line]
            depth = brace_delta(raw_line)
            i += 1
            while i < len(inner) and depth > 0:
                collected.append(inner[i])
                depth += brace_delta(inner[i])
                i += 1
            raw = "\n".join(collected)
            route["directives"].append({"type": "raw_block", "raw": raw})
            route["complex"] = True
            continue

        known = {
            "file_server",
            "redir",
            "respond",
            "encode",
            "header",
            "php_fastcgi",
            "root",
            "try_files",
            "request_body",
            "basic_auth",
            "log",
            "handle_path",
            "rewrite",
        }
        if keyword in known:
            route["directives"].append({"type": keyword, "args": rest, "raw": raw_line})
        else:
            route["directives"].append({"type": "raw", "raw": raw_line})
            route["complex"] = True
        i += 1

    return route


def parse_config(content: str) -> dict[str, Any]:
    prefix, blocks = split_top_level_blocks(content)
    preserved = []
    routes = []
    for block in blocks:
        if is_global_or_snippet(block["header"]):
            preserved.append(block)
        else:
            routes.append(parse_site_block(block["source"]))
    return {"prefix": "\n".join(prefix).strip(), "preserved": preserved, "routes": routes, "raw": content}


def rebuild_config(parsed: dict[str, Any], routes: list[dict[str, Any]]) -> str:
    chunks: list[str] = []
    if parsed.get("prefix"):
        chunks.append(parsed["prefix"])
    chunks.extend(block["source"].rstrip() for block in parsed["preserved"])
    chunks.extend(route["source"].rstrip() for route in routes)
    return "\n\n".join(chunk for chunk in chunks if chunk.strip())


def find_route_index(routes: list[dict[str, Any]], original_id: str) -> int:
    for index, route in enumerate(routes):
        if route["id"] == original_id:
            return index
    raise HTTPException(status_code=404, detail="Route not found")


@app.get("/", response_class=HTMLResponse)
async def home(caddedit_auth: str | None = Cookie(default=None)) -> str:
    if caddedit_auth and hmac.compare_digest(caddedit_auth, auth_token()):
        return HTML
    return LOGIN_HTML


@app.post("/api/auth/login")
async def login(payload: LoginPayload, response: Response) -> dict[str, Any]:
    """Browser login - validates the password and sets an HttpOnly session
    cookie. Used by the web UI only; the CLI uses /api/auth/token instead."""
    if not hmac.compare_digest(payload.password, ADMIN_PASSWORD):
        raise HTTPException(status_code=401, detail="Wrong password")
    response.set_cookie(
        AUTH_COOKIE,
        auth_token(),
        httponly=True,
        secure=True,
        samesite="lax",
        max_age=60 * 60 * 24 * 14,
    )
    return {"ok": True}


@app.post("/api/auth/logout")
async def logout(response: Response) -> dict[str, Any]:
    response.delete_cookie(AUTH_COOKIE)
    return {"ok": True}


@app.post("/api/auth/token")
async def issue_token(payload: LoginPayload) -> dict[str, Any]:
    """Exchange the unlock password for the permanent API token, with no
    cookie involved. This is what `caddedit config login` calls so the CLI
    never has to store the raw password on disk - only the derived token."""
    if not hmac.compare_digest(payload.password, ADMIN_PASSWORD):
        raise HTTPException(status_code=401, detail="Wrong password")
    return {"ok": True, "token": auth_token()}


@app.get("/api/auth/token", dependencies=[Depends(require_auth)])
async def read_token() -> dict[str, Any]:
    """Lets an already-logged-in browser session fetch the same permanent
    token (the cookie is HttpOnly, so JS can't read it directly). Powers the
    web UI's "CLI Connect" panel without asking the person to retype their
    password."""
    return {"ok": True, "token": auth_token()}


@app.get("/api/auth/verify", dependencies=[Depends(require_auth)])
async def verify_auth() -> dict[str, Any]:
    """Cheap authenticated ping. The CLI calls this once on connect so a
    stale/garbled token fails fast with a clear message instead of failing
    deep inside whatever command was actually requested."""
    return {"ok": True}


@app.get("/api/config", dependencies=[Depends(require_auth)])
async def api_config() -> dict[str, Any]:
    parsed = parse_config(read_caddyfile())
    parsed["routes"] = read_vhost_routes()
    parsed["disable_ai"] = DISABLE_AI
    parsed["site_domain_hint"] = SITE_DOMAIN_HINT
    return parsed



@app.put("/api/config", dependencies=[Depends(require_auth)])
async def update_config(payload: ConfigPayload) -> dict[str, Any]:
    write_caddyfile(payload.source)
    return {"ok": True}



@app.post("/api/routes", dependencies=[Depends(require_auth)])
async def create_route(payload: RoutePayload) -> dict[str, Any]:
    new_route = parse_site_block(payload.source)
    ensure_vhost_dirs()
    path = route_path(new_route["domain"], payload.status)
    (ENABLED_DIR / path.name).unlink(missing_ok=True)
    (DISABLED_DIR / path.name).unlink(missing_ok=True)
    path.write_text(payload.source.rstrip() + "\n", encoding="utf-8")
    run_caddy_reload()
    return {"ok": True, "route": new_route}




@app.put("/api/routes/{route_id}", dependencies=[Depends(require_auth)])
async def update_route(route_id: str, payload: RoutePayload) -> dict[str, Any]:
    old = find_vhost_route(route_id)
    old_path = Path(old["file"])
    updated = parse_site_block(payload.source)
    new_path = route_path(updated["domain"], payload.status)
    old_path.unlink(missing_ok=True)
    (ENABLED_DIR / new_path.name).unlink(missing_ok=True)
    (DISABLED_DIR / new_path.name).unlink(missing_ok=True)
    new_path.write_text(payload.source.rstrip() + "\n", encoding="utf-8")
    run_caddy_reload()
    return {"ok": True, "route": updated}




@app.delete("/api/routes/{route_id}", dependencies=[Depends(require_auth)])
async def delete_route(route_id: str) -> dict[str, Any]:
    removed = find_vhost_route(route_id)
    Path(removed["file"]).unlink(missing_ok=True)
    run_caddy_reload()
    return {"ok": True, "removed": removed["domain"]}


@app.post("/api/routes/{route_id}/toggle", dependencies=[Depends(require_auth)])
async def toggle_route(route_id: str) -> dict[str, Any]:
    route = find_vhost_route(route_id)
    old_path = Path(route["file"])
    target_dir = DISABLED_DIR if route["status"] == "ON" else ENABLED_DIR
    target_path = target_dir / old_path.name
    old_path.rename(target_path)
    run_caddy_reload()
    return {"ok": True, "status": "OFF" if route["status"] == "ON" else "ON"}


@app.post("/api/reload", dependencies=[Depends(require_auth)])
async def reload_caddy() -> dict[str, Any]:
    return run_caddy_reload()


@app.post("/api/hardcoded-rules", dependencies=[Depends(require_auth)])
async def save_hardcoded_rule(payload: HardcodedRulePayload) -> dict[str, Any]:
    rules = read_learned_rules()
    parsed = dict(payload.parsed)
    parsed["parser"] = "learned"
    rules[source_key(payload.source)] = parsed
    write_learned_rules(rules)
    return {"ok": True, "key": source_key(payload.source)}


@app.delete("/api/hardcoded-rules", dependencies=[Depends(require_auth)])
async def delete_hardcoded_rule(payload: AiParsePayload) -> dict[str, Any]:
    rules = read_learned_rules()
    removed = rules.pop(source_key(payload.source), None)
    write_learned_rules(rules)
    return {"ok": True, "removed": bool(removed)}


@app.post("/api/ai/parse", dependencies=[Depends(require_auth)])
async def ai_parse(payload: AiParsePayload) -> dict[str, Any]:
    if DISABLE_AI:
        raise HTTPException(status_code=400, detail="AI features are disabled")
    if not COHERE_API_KEY:
        raise HTTPException(status_code=400, detail="COHERE_API_KEY is not configured")


    schema = {
        "type": "object",
        "properties": {
            "domain": {"type": "string"},
            "request_watch_log": {"type": "boolean"},
            "reverse_proxy": {"type": "string"},
            "tls": {
                "type": "object",
                "properties": {
                    "mode": {"type": "string"},
                    "raw": {"type": "string"},
                },
                "required": ["mode", "raw"],
            },
            "directives": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "type": {"type": "string"},
                        "args": {"type": "string"},
                        "raw": {"type": "string"},
                    },
                    "required": ["type"],
                },
            },
            "safe_for_gui": {"type": "boolean"},
            "notes": {"type": "string"},
        },
        "required": ["domain", "request_watch_log", "reverse_proxy", "tls", "directives", "safe_for_gui", "notes"],
    }
    body = {
        "model": COHERE_MODEL,
        "messages": [
            {
                "role": "system",
                "content": (
                    "Parse this Caddyfile site block into GUI fields. Preserve every original string. "
                    "Set request_watch_log true only for import request_watch_log. Put simple reverse_proxy targets "
                    "in reverse_proxy. Keep complex reverse_proxy blocks, heredocs, PHP blocks, matchers, and unknown "
                    "directives as raw/raw_block directives."
                ),
            },
            {"role": "user", "content": payload.source},
        ],
        "response_format": {"type": "json_object", "schema": schema},
        "temperature": 0,
    }
    async with httpx.AsyncClient(timeout=30) as client:
        response = await client.post(
            "https://api.cohere.com/v2/chat",
            headers={"Authorization": f"Bearer {COHERE_API_KEY}", "Content-Type": "application/json"},
            json=body,
        )
    if response.status_code >= 400:
        raise HTTPException(status_code=502, detail=f"Cohere v2 parsing failed: {response.text}")
    data = response.json()
    try:
        text = data["message"]["content"][0]["text"]
        parsed = json.loads(text)
    except Exception as exc:
        raise HTTPException(status_code=502, detail=f"Cohere returned an unreadable parse: {exc}") from exc
    return {"ok": True, "parsed": parsed}




if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host=os.getenv("HOST", "0.0.0.0"), port=int(os.getenv("PORT", "29048")))