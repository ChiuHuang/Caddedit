<div align="center">

# Caddedit

> 🌐 English | [繁體中文](README.zh-TW.md)

**Split, inspect and toggle Caddy site blocks — without pain.**

One static binary. No Python. No daemon. Your Caddyfile stays the source of truth.

[![Release](https://img.shields.io/github/v/release/ChiuHuang/Caddedit?style=flat-square&color=2563eb)](https://github.com/ChiuHuang/Caddedit/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/ChiuHuang/Caddedit/ci.yml?branch=main&style=flat-square&label=CI)](https://github.com/ChiuHuang/Caddedit/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-dea584?style=flat-square&logo=rust)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-3fb950?style=flat-square)](LICENSE)

```bash
curl -sSL https://raw.githubusercontent.com/ChiuHuang/Caddedit/main/install.sh | sudo bash
```

</div>

---

```console
$ caddedit ls
      DOMAINS                                   TYPE    UPSTREAM                        TLS
● on   app.example.com                           proxy   localhost:3000                  -
● on   grafana.example.com, metrics.example.com  proxy   localhost:3333, localhost:3334  internal
○ off  legacy.example.net:8443                   static  -                               acme email (me@example.com)
● on   www.example.net                           static  -                               dns challenge (cloudflare)
● on   edge.example.io                           raw     h2c://backend:9000              -
```

## Why

A monolithic `Caddyfile` turns into a dumping ground; web panels turn your
config into form fields that fight real syntax. **Caddedit takes neither side:**

- keeps one file per site under `vhosts/enabled/` and `vhosts/disabled/`,
  moved around **byte-for-byte** — tabs, comments and heredocs stay untouched
- anything it can't fully understand is shown honestly as `raw`, never mangled
- every mutation runs `caddy validate` *before* it can go live, and rolls back
  automatically when validation or reload fails
- single static musl binary — `scp` it to any server and run

## Commands

| | | |
| --- | --- | --- |
| `caddedit init` | split a monolithic Caddyfile | `--force` re-splits |
| `caddedit ls --json` | all routes at a glance | scriptable output |
| `caddedit show [domain]` | print one site block | bare → interactive picker |
| `caddedit new app.com` | scaffold a route | wizard or flags |
| `caddedit on / off <domain>...` | park & restore routes | validated + reload |
| `caddedit rm [domain]` | soft-delete → backups/ | never hard-deletes |
| `caddedit edit [domain]` | `$EDITOR`, checked on exit | |
| `caddedit check` | validate everything | exit code for cron |
| `caddedit reload` | reload caddy | honors custom command |
| `caddedit serve` | optional MDUI dashboard | embedded, no CDN |

Run **bare `caddedit`** for an interactive TUI browser:

| Key | Action | Key | Action |
| --- | --- | --- | --- |
| `j/k` | move | `e` | edit block |
| `space` | toggle | `d` + `y` | remove |
| `r` | reload caddy | `q` | quit |

## How it works

```
/etc/caddy/Caddyfile          global options + snippets + import line
/etc/caddy/vhosts/enabled/    live site blocks        — one file per route
/etc/caddy/vhosts/disabled/   parked routes           — toggling just moves files
/etc/caddy/backups/           timestamped backups + soft-deleted routes
```

## Web dashboard

```bash
CADDEDIT_PASSWORD=secret caddedit serve --host 127.0.0.1 --port 29048
```

Material Design 3 interface compiled into the binary — route list with
toggles, raw editor with inline validation errors, route creation with TLS
presets, dark/light theme. Works offline.

<details>
<summary><strong>Environment variables</strong></summary>

| Variable | Default | Purpose |
| --- | --- | --- |
| `CADDYFILE_PATH` | `/etc/caddy/Caddyfile` | main config path |
| `VHOSTS_DIR` | `<config parent>/vhosts` | enabled/disabled root |
| `CADDY_BACKUP_DIR` | `<config parent>/backups` | backups |
| `CADDY_BIN` | `caddy` | binary for validate/reload |
| `CADDEDIT_RELOAD_COMMAND` | `caddy reload --config <path>` | custom reload |
| `CADDEDIT_PASSWORD` | *(unset = open)* | dashboard password |

</details>

## Migrating from the legacy Python webui?

The `vhosts/` layout is identical — install the new binary, run `caddedit ls`
to confirm it sees your routes, then stop the old service. Full steps:
[`cli/README.md`](cli/README.md#migrating-from-the-legacy-python-webui).
The legacy FastAPI code was removed; it lives on in git history if you ever
need it.
