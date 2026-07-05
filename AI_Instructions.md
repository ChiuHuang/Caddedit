# Caddedit CLI — AI Agent Instructions

You have access to the `caddedit` command. It is a **client** for the
Caddedit API (the Caddyfile GUI editor) — it does not manage the server
process itself, it manages *routes* on a running Caddedit server over
HTTP. Its connection settings (server URL + unlock password) live in its
own local config file (`~/.config/caddedit/cli.json`), separate from the
server's `.env`.

## Commands available to you

```
caddedit config                        # show current CLI config (password masked)
caddedit config export                 # print a shareable connection string (base64 of url+password)
caddedit config <connection-string>    # import a connection string from 'export' on another machine
caddedit config set-url <url>          # set only the server URL
caddedit config set-password [PASS]    # set only the password (prompts if omitted)

caddedit list                          # list all routes
caddedit show <id>                     # show one route's full detail/source
caddedit add [--domain D] [--target T] [--tls MODE] [--raw BLOCK] [--disabled]
caddedit edit <id> [--target T] [--tls MODE] [--raw BLOCK] [--enable|--disable]
caddedit toggle <id>                   # flip a route on/off
caddedit delete <id> [-y]              # delete a route (asks to confirm unless -y)
caddedit reload                        # trigger a Caddy reload
caddedit url                           # print the configured server URL
```

Running `caddedit` with no arguments drops into an interactive numbered
menu that covers the same actions — use the full commands above instead
when you (the agent) are the one driving, since they're scriptable and
don't require a TTY prompt loop.

## How to use each command

- **`list`** — run this first whenever the user asks what routes exist, or
  before adding/editing one, to get the current `id` values (they're
  required for `show`/`edit`/`toggle`/`delete`).
- **`add`** — use `--domain` + `--target` for a simple reverse-proxy route.
  Use `--raw` only when the user has given you an exact Caddy site block to
  use verbatim (e.g. with directives the simple flags don't cover).
- **`edit`** — fetches the existing route first, so omitted flags keep
  their current value; only pass the flags that should change.
- **`delete`** — destructive. Confirm with the user before passing `-y`
  to skip the prompt, unless they were already explicit about which route
  to delete.
- **`reload`** — use after a batch of changes, not after every single
  `add`/`edit` (the API already reloads Caddy on each route mutation, so
  this is mainly useful if the user edited the raw Caddyfile separately).

## Safety rules

1. Never print the unlock password anywhere except in direct response to
   the person operating this host. The same applies to a connection
   string from `config export` - it's just the URL+password base64'd
   together, not encrypted, so treat it exactly as sensitively as the
   password itself.
2. `delete` is irreversible from the CLI's perspective — always confirm
   intent if there's any ambiguity about which route the user means.
3. If a command fails with a login/connection error, surface that to the
   user (it usually means the CLI's stored URL or password is stale, or
   the server is down) rather than retrying repeatedly.
4. Don't invent route IDs — always get them from `caddedit list` /
   `caddedit show` first.

## Example interactions

| User says | You run |
|---|---|
| "What routes do I have?" | `caddedit list` |
| "Add foo.example.com pointing to 127.0.0.1:8080" | `caddedit add --domain foo.example.com --target 127.0.0.1:8080` |
| "Turn off the staging route" | `caddedit list` (find id) → `caddedit toggle <id>` |
| "Delete the old test domain" | `caddedit list` (find id) → confirm → `caddedit delete <id>` |
| "Point the CLI at my new server" | `caddedit config set-url http://1.2.3.4:29048` |