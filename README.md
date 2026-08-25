# Caddedit

Caddy route management tools.

```
Caddedit/
├── cli/          # current: single-binary Rust CLI (+ optional embedded dashboard)
├── manager.py    # legacy FastAPI webui — superseded, kept for reference
└── install.sh    # one-line installer for the CLI binary
```

## Install

```bash
curl -sSL https://raw.githubusercontent.com/ChiuHuang/Caddedit/main/install.sh | sudo bash
```

## The CLI

Split, inspect and toggle Caddy site blocks without pain:

```bash
caddedit init                 # backup + split monolithic Caddyfile into vhosts/
caddedit ls                   # ● on  app.example.com  proxy  localhost:3000
caddedit off app.example.com  # park a route (validated, then reload)
caddedit                      # interactive TUI browser
caddedit serve                # optional MDUI web dashboard
```

Full command reference, TUI keys, env vars and the web dashboard guide:
[`cli/README.md`](cli/README.md).

Every mutation runs `caddy validate` before it can go live and auto-rolls back
on failure. Site blocks are moved byte-for-byte — tabs, comments and heredocs
stay exactly as written.

## Migrating from the Python webui

The `vhosts/enabled` + `vhosts/disabled` layout is identical — existing installs
work with zero conversion. Steps: install the new binary, run `caddedit ls` to
confirm it sees your routes, then disable the old systemd service and remove
`/opt/caddedit`. Details in [`cli/README.md`](cli/README.md#migrating-from-caddedit-python-webui).
