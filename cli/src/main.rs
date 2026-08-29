mod auth_client;
mod caddy;
mod caddyfile;
mod config;
mod fsutil;
mod ops;
mod picker;
mod selfupdate;
mod server;
mod tui;
mod vhost;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::shells::Shell;
use config::Paths;
use owo_colors::OwoColorize;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "caddedit",
    version,
    about = "Split, inspect and toggle Caddy site blocks without pain",
    after_help = "Environment: CADDYFILE_PATH, VHOSTS_DIR, CADDY_BACKUP_DIR,\n                 CADDY_BIN, CADDEDIT_RELOAD_COMMAND, CADDEDIT_PASSWORD"
)]
struct Cli {
    /// Main Caddyfile (env: CADDYFILE_PATH; default /etc/caddy/Caddyfile)
    #[arg(long, short = 'c', global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    /// vhosts root containing enabled/ + disabled/ (env: VHOSTS_DIR)
    #[arg(long, global = true, value_name = "DIR")]
    vhosts_dir: Option<PathBuf>,

    /// Interactive route browser when omitted
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Split the monolithic Caddyfile into per-site vhost files
    Init {
        #[arg(long)]
        force: bool,
        #[arg(long)]
        no_reload: bool,
    },
    /// List every route with status, type, upstream and TLS
    #[command(alias = "list")]
    Ls {
        #[arg(long)]
        json: bool,
        /// Substring filter across domain/upstream/tls/directives
        query: Option<String>,
    },
    /// Print one route's raw site block (interactive picker when omitted)
    Show { domain: Option<String> },
    /// Enable routes
    On {
        domains: Vec<String>,
        #[arg(long)]
        no_reload: bool,
    },
    /// Disable routes (files move to disabled/)
    Off {
        domains: Vec<String>,
        #[arg(long)]
        no_reload: bool,
    },
    /// Remove a route (moved to backups/, never hard-deleted)
    Rm {
        domain: Option<String>,
        #[arg(short, long)]
        yes: bool,
        #[arg(long)]
        no_reload: bool,
    },
    /// Open $EDITOR on a route's block, validate on exit
    Edit {
        domain: Option<String>,
        #[arg(long)]
        no_reload: bool,
    },
    /// Validate main config + every enabled route
    Check,
    /// Reload caddy
    Reload,
    /// Scaffold a new site block (interactive when flags are omitted)
    New {
        /// Domains, comma separated
        domains: Option<String>,
        #[arg(long)]
        upstream: Option<String>,
        #[arg(long)]
        tls: Option<String>,
        /// Add `import request_watch_log` to the block
        #[arg(long)]
        watch_log: bool,
        #[arg(long)]
        no_reload: bool,
    },
    /// Generate shell completions (bash, zsh, fish, powershell)
    Completions { shell: Shell },
    /// Serve the web dashboard (embedded, MDUI)
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(short, long, default_value_t = 29048)]
        port: u16,
    },
    /// Login to a remote caddedit server using a refresh token (User-Agent: caddedit-cli)
    Login {
        /// Server URL, e.g. https://caddedit.example.com
        #[arg(long)]
        url: Option<String>,
        /// One-time refresh token from Settings → CLI Access
        #[arg(long)]
        refresh_token: Option<String>,
        /// Save access token to config file (~/.config/caddedit/cli.json) (default true, use --no-save to print only)
        #[arg(long)]
        no_save: bool,
    },
    /// Clear stored CLI tokens
    Logout,
    /// Show stored CLI config (url + masked tokens)
    Config,
    /// Update caddedit binary to latest GitHub release (CLI self-update, distinct from web dashboard)
    Update {
        /// Only check for updates, don't install
        #[arg(long)]
        check: bool,
        /// Release channel: stable or nightly
        #[arg(long, default_value = "stable")]
        channel: String,
        /// Auto confirm without prompt
        #[arg(long)]
        yes: bool,
    },
}

fn main() -> std::process::ExitCode {
    // Rust ignores SIGPIPE by default; `caddedit ls | head` must not panic.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();
    let paths = Paths::resolve(cli.config.clone(), cli.vhosts_dir.clone());

    let result = match cli.cmd {
        None => tui::run(&paths),
        Some(cmd) => match cmd {
            Cmd::Init { force, no_reload } => ops::init::run(&paths, force, no_reload),
            Cmd::Ls { json, query } => ops::ls::run(&paths, json, query.as_deref()),
            Cmd::Show { domain } => ops::show::run(&paths, domain.as_deref()),
            Cmd::On { domains, no_reload } => ops::toggle::enable(&paths, &domains, no_reload),
            Cmd::Off { domains, no_reload } => ops::toggle::disable(&paths, &domains, no_reload),
            Cmd::Rm {
                domain,
                yes,
                no_reload,
            } => ops::toggle::remove(&paths, domain.as_deref(), yes, no_reload),
            Cmd::Edit { domain, no_reload } => ops::edit::run(&paths, domain.as_deref(), no_reload),
            Cmd::New {
                domains,
                upstream,
                tls,
                watch_log,
                no_reload,
            } => ops::new::run(
                &paths,
                domains.as_deref(),
                upstream.as_deref(),
                tls.as_deref(),
                watch_log,
                no_reload,
            ),
            Cmd::Completions { shell } => {
                let mut cmd = Cli::command();
                clap_complete::generate(shell, &mut cmd, "caddedit", &mut std::io::stdout());
                Ok(())
            }
            Cmd::Check => match ops::check::run(&paths) {
                Ok(true) => Ok(()),
                Ok(false) => std::process::exit(1),
                Err(e) => Err(e),
            },
            Cmd::Reload => ops::reload::run(&paths),
            Cmd::Serve { host, port } => {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                return match rt.block_on(server::run(&host, port, paths)) {
                    Ok(()) => std::process::ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("{}", format!("error: {e:#}").red());
                        std::process::ExitCode::FAILURE
                    }
                };
            }
            Cmd::Login {
                url,
                refresh_token,
                no_save,
            } => {
                // interactive TUI when args omitted and stdin is a TTY
                let use_tui = url.is_none() || refresh_token.is_none();
                if use_tui
                    && std::io::IsTerminal::is_terminal(&std::io::stdin())
                    && std::io::IsTerminal::is_terminal(&std::io::stdout())
                {
                    auth_client::run_login_tui(!no_save)
                } else {
                    let u = url.as_deref().unwrap_or("");
                    let r = refresh_token.as_deref().unwrap_or("");
                    if u.is_empty() || r.is_empty() {
                        Err(anyhow::anyhow!(
                            "url and --refresh-token are required in non-interactive shells; use `caddedit login --url <url> --refresh-token <token>` or run in a terminal for interactive login"
                        ))
                    } else {
                        auth_client::run_login(u, r, !no_save)
                    }
                }
            }
            Cmd::Logout => auth_client::run_logout(),
            Cmd::Config => auth_client::run_config_show(),
            Cmd::Update {
                check,
                channel,
                yes,
            } => {
                // launch interactive TUI when in a TTY and no explicit non-interactive flag
                let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin())
                    && std::io::IsTerminal::is_terminal(&std::io::stdout());
                if interactive && !check && !yes {
                    // if channel was left as default and no install intent, show TUI
                    // TUI handles channel switching + check + install interactively
                    ops::update::run_tui(&channel)
                } else {
                    ops::update::run(check, &channel, yes)
                }
            }
        },
    };

    if let Err(e) = result {
        eprintln!("{}", format!("error: {e:#}").red().bold());
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
