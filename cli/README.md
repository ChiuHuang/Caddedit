<div align="center">

# caddedit (CLI)

**Split, inspect and toggle Caddy site blocks — without pain.**

[![Release](https://img.shields.io/github/v/release/ChiuHuang/Caddedit?style=flat-square&color=2563eb)](https://github.com/ChiuHuang/Caddedit/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/ChiuHuang/Caddedit/ci.yml?branch=main&style=flat-square&label=CI)](https://github.com/ChiuHuang/Caddedit/actions/workflows/ci.yml)

Part of the [Caddedit](https://github.com/ChiuHuang/Caddedit#readme) monorepo.

</div>

---

```
$ caddedit ls
      DOMAINS                                   TYPE    UPSTREAM                        TLS
● on   app.example.com                           proxy   localhost:3000                  -
● on   grafana.example.com, metrics.example.com  proxy   localhost:3333, localhost:3334  internal
○ off  legacy.example.net:8443                   static  -                               acme email (me@example.com)
● on   www.example.net                           static  -                               dns challenge (cloudflare)
● on   edge.example.io                           raw     h2c://backend:9000              -
```

## How it works

Caddedit splits your monolithic `Caddyfile` into per-site files under
`vhosts/enabled/*.caddy` and `vhosts/disabled/*.caddy`, rewriting the main config
to just global options + an `import` line:

```
/etc/caddy/Caddyfile          # global options + snippets + import
/etc/caddy/vhosts/enabled/    # live site blocks (one file per route)
/etc/caddy/vhosts/disabled/   # parked routes
/etc/caddy/backups/           # timestamped backups, soft-deleted routes
```

Every mutation is validated with `caddy validate` *before* it can go live, and a
failed reload never leaves you with a broken config. Site blocks are moved
byte-for-byte — tabs, comments and heredocs stay exactly as you wrote them.
Anything the analyzer can't fully understand shows up honestly as `raw` instead
of being mangled into form fields.

## Install

```bash
curl -sSL https://raw.githubusercontent.com/ChiuHuang/Caddedit/main/install.sh | sudo bash
```

or with Rust:

```bash
cargo install --git https://github.com/ChiuHuang/Caddedit --path cli
```

## Commands

| Command | What it does |
| --- | --- |
| `caddedit init` | Backup + split the monolithic Caddyfile (`--force` re-splits) |
| `caddedit ls [--json] [query]` | All routes: status, type, upstreams, TLS — substring filter included |
| `caddedit show [domain]` | Print one raw site block |
| `caddedit new app.com --upstream localhost:3000 --tls internal_explicit` | scaffold a route (--watch-log adds the logging snippet; omit flags for wizard) |
| `caddedit on/off <domain>...` | Enable/disable (moves between folders + reload) |
| `caddedit rm [domain] [-y]` | Soft-delete into backups/ |
| `caddedit edit [domain]` | `$EDITOR` on the block, validated on exit |
| `caddedit check` | Validate main config + every enabled route |
| `caddedit reload` | Reload caddy |
| `caddedit completions <bash\|zsh\|fish\|powershell>` | Shell completions |

Commands that take a domain (`show`, `on`, `off`, `rm`, `edit`) open an
interactive picker when you omit it. In scripts (no TTY) they fail fast and
list the available route names instead of hanging.

### TUI

Run bare `caddedit` for an interactive browser:

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | Move selection |
| `space` / `enter` | Toggle on/off (validated first) |
| `e` | Edit block in `$EDITOR` |
| `d`, then `y` | Remove (soft-delete) |
| `/` | live filter |
| `r` / R | Reload caddy / refresh list |
| `q` | Quit |
### Web dashboard

```bash
CADDEDIT_PASSWORD=secret caddedit serve --host 127.0.0.1 --port 29048
```

Embedded Material Design UI (MDUI 2), no external assets. Route list with
toggles, raw editor with live validation, route creation with TLS presets,
reload button, dark/light theme.

## Environment variables

| Variable | Default | Purpose |
| --- | --- | --- |
| `CADDYFILE_PATH` | `/etc/caddy/Caddyfile` | Main config path |
| `VHOSTS_DIR` | `<config parent>/vhosts` | enabled/disabled root |
| `CADDY_BACKUP_DIR` | `<config parent>/backups` | Backups |
| `CADDY_BIN` | `caddy` | Binary used for validate/reload |
| `CADDEDIT_RELOAD_COMMAND` | `caddy reload --config <path>` | Custom reload command |
| `CADDEDIT_PASSWORD` | *(unset = no auth)* | Web dashboard password |

## Migrating from the legacy Python webui

The file layout is intentionally identical — the new CLI reads the same
`vhosts/enabled` + `vhosts/disabled` tree the old FastAPI app created, so an
existing install keeps working with zero conversion:

```bash
# 1. install the new binary
curl -sSL https://raw.githubusercontent.com/ChiuHuang/Caddedit/main/install.sh | sudo bash

# 2. verify it sees your routes (same env vars as the old .env)
CADDYFILE_PATH=/etc/caddy/Caddyfile caddedit ls

# 3. stop + remove the old service
sudo systemctl disable --now caddedit.service
sudo rm -rf /opt/caddedit

# 4. optional: bring back a dashboard on the same port
CADDEDIT_PASSWORD=yourpassword caddedit serve --host 127.0.0.1 --port 29048 \
    # run under systemd if you want it always-on
```

Notes:
- Env var names are unchanged (`CADDYFILE_PATH`, `VHOSTS_DIR`,
  `CADDY_BACKUP_DIR`, `CADDEDIT_PASSWORD`) — copy them from the old `.env`.
- Never re-run `caddedit init` on an already-split config; it detects the
  existing import line and refuses unless `--force`.
- The Cohere AI parsing layer is gone by design — the CLI reads raw syntax,
  so there is nothing left to translate.
- If you used `caddy reload --config ...` customizations, set
  `CADDEDIT_RELOAD_COMMAND` to the same string.

## Development

```bash
cargo test            # unit + e2e (real binary against scratch dirs)
cargo clippy --all-targets -- -D warnings
cargo run -- --config fixtures/sample.Caddyfile ls   # copy the fixture somewhere mutable first
```

Release builds embed `web/` into the binary. Cross-compilation is handled by
the `release.yml` workflow (zigbuild for static musl Linux builds).
