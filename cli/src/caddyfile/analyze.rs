//! Structural analysis of a single site block: which directives it uses,
//! whether we can render a "simple" structured summary, upstreams, TLS mode,
//! plus human-readable per-directive detail lines for the UIs.

use super::lexer::{tokenize, TokKind, Token};

#[derive(Debug, Clone, PartialEq)]
pub struct DirNode {
    pub name: String,
    pub args: Vec<String>,
    pub children: Vec<DirNode>,
}

/// Directives we understand well enough to call a site "simple".
const KNOWN_DIRECTIVES: &[&str] = &[
    "reverse_proxy",
    "root",
    "file_server",
    "tls",
    "redir",
    "respond",
    "abort",
    "error",
    "handle",
    "handle_path",
    "handle_errors",
    "route",
    "encode",
    "header",
    "log",
    "log_skip",
    "log_append",
    "skip_log",
    "basic_auth",
    "basicauth",
    "forward_auth",
    "php_fastcgi",
    "request_body",
    "uri",
    "rewrite",
    "method",
    "try_files",
    "templates",
    "vars",
    "import",
    "push",
    "map",
    "fs",
];

/// Max detail lines rendered before collapsing into "+N more".
const MAX_DETAILS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteKind {
    Proxy,
    Static,
    Php,
    Other,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TlsMode {
    Internal,
    AcmeEmail,
    Dns,
    Manual,
    Custom,
}

impl TlsMode {
    pub fn label(&self) -> &'static str {
        match self {
            TlsMode::Internal => "internal",
            TlsMode::AcmeEmail => "acme email",
            TlsMode::Dns => "dns challenge",
            TlsMode::Manual => "cert/key",
            TlsMode::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TlsInfo {
    pub mode: TlsMode,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SiteInfo {
    pub addresses: Vec<String>,
    pub kind: SiteKind,
    pub upstreams: Vec<String>,
    pub tls: Option<TlsInfo>,
    /// Directive names at the top level of the block.
    pub directives: Vec<String>,
    /// One compact human-readable line per recognized directive.
    pub details: Vec<String>,
    /// `import request_watch_log` present (legacy webui logging snippet).
    pub watch_log: bool,
    /// True when the legacy GUI would have refused structured editing:
    /// nested `reverse_proxy`/`tls` blocks beyond the known presets, or any
    /// unknown directive. Drives the `raw` classification.
    pub complex: bool,
}

pub fn analyze_site(src_slice: &str) -> SiteInfo {
    // Parse only the inner tokens of the (single) site block in this slice.
    let toks = tokenize(src_slice);
    let mut addresses = Vec::new();
    let mut inner_start = 0usize;

    for (idx, t) in toks.iter().enumerate() {
        if t.is(TokKind::OpenBrace) {
            addresses = toks[..idx]
                .iter()
                .filter(|t| t.is_wordish())
                .map(|t| {
                    t.text
                        .trim_matches(|c| c == ',' || c == ';')
                        .trim()
                        .to_string()
                })
                .filter(|s| !s.is_empty())
                .collect();
            inner_start = idx + 1;
            break;
        }
    }

    let dirs = parse_directives(&toks[inner_start..]);
    let directives: Vec<String> = dirs.iter().map(|d| d.name.clone()).collect();

    let is_simple = directives
        .iter()
        .all(|n| KNOWN_DIRECTIVES.contains(&n.as_str()));

    let tls = dirs
        .iter()
        .find(|d| d.name == "tls")
        .map(tls_from_directive);

    // ---- legacy hardcoded-parser semantics (port of manager.py) ----
    let mut watch_log = false;
    let mut complex = !is_simple;
    for d in &dirs {
        match d.name.as_str() {
            "import" if d.args.first().map(|a| a == "request_watch_log") == Some(true) => {
                watch_log = true;
            }
            // multi-target / option blocks can't be round-tripped by the
            // structured editor -> raw, exactly like the old GUI
            "reverse_proxy" if !d.children.is_empty() => complex = true,
            "tls" if d.args.is_empty() => {
                // block form: cloudflare dns preset is understood, anything
                // else (other providers, custom issuers) is complex
                let dns = d.children.iter().find(|c| c.name == "dns");
                let cf = dns
                    .and_then(|c| c.args.first())
                    .map(|p| p.eq_ignore_ascii_case("cloudflare"))
                    .unwrap_or(false);
                if !cf {
                    complex = true;
                }
            }
            _ => {}
        }
    }

    let kind = if complex {
        SiteKind::Raw
    } else if dirs.iter().any(|d| d.name == "reverse_proxy") {
        SiteKind::Proxy
    } else if dirs.iter().any(|d| d.name == "php_fastcgi") {
        SiteKind::Php
    } else if dirs
        .iter()
        .any(|d| d.name == "root" || d.name == "file_server")
    {
        SiteKind::Static
    } else {
        SiteKind::Other
    };

    // Upstreams live on the `reverse_proxy` line itself; sub-block entries
    // (`header_up`, transport options...) must not leak into this list.
    let upstreams: Vec<String> = dirs
        .iter()
        .filter(|d| d.name == "reverse_proxy")
        .flat_map(|d| d.args.iter())
        .filter(|a| !a.starts_with('-') && !a.contains('='))
        .cloned()
        .collect();

    let mut details: Vec<String> = Vec::new();
    for d in &dirs {
        if let Some(line) = summarize_directive(d) {
            details.push(line);
        }
    }
    if details.len() > MAX_DETAILS {
        let hidden = details.len() - MAX_DETAILS;
        details.truncate(MAX_DETAILS);
        details.push(format!("+{hidden} more"));
    }

    SiteInfo {
        addresses,
        kind,
        upstreams,
        tls,
        directives,
        details,
        watch_log,
        complex,
    }
}

fn join_args(args: &[String], n: usize) -> String {
    args.iter().take(n).cloned().collect::<Vec<_>>().join(" ")
}

/// Compact one-line summary of a directive, or None to stay quiet
/// (`reverse_proxy` and `tls` have their own dedicated columns).
fn summarize_directive(d: &DirNode) -> Option<String> {
    Some(match d.name.as_str() {
        "reverse_proxy" | "tls" => return None,

        "root" | "fs" => format!("{} {}", d.name, join_args(&d.args, 4)),
        "file_server" => {
            if d.children.iter().any(|c| c.name == "browse") || d.args.iter().any(|a| a == "browse")
            {
                "file_server (browse)".to_string()
            } else {
                "file_server".to_string()
            }
        }

        "redir" => format!("redir {}", join_args(&d.args, 3)),
        "respond" => format!("respond {}", join_args(&d.args, 2)),
        "abort" => "abort".to_string(),
        "error" => format!("error {}", join_args(&d.args, 2)),

        "handle" => format!("handle {} ({} dir)", join_args(&d.args, 1), count_dirs(d)),
        "handle_path" => format!(
            "handle_path {} ({} dir)",
            join_args(&d.args, 1),
            count_dirs(d)
        ),
        "handle_errors" => format!("handle_errors {}", join_args(&d.args, 1)),
        "route" => format!("route ({} dir)", count_dirs(d)),

        "encode" => {
            let encodings: Vec<String> = if d.args.is_empty() {
                d.children.iter().map(|c| c.name.clone()).collect()
            } else {
                d.args.clone()
            };
            if encodings.is_empty() {
                "encode".to_string()
            } else {
                format!("encode {}", encodings.join("+"))
            }
        }

        "header" => {
            let mut parts: Vec<String> = Vec::new();
            if !d.args.is_empty() {
                parts.push(join_args(&d.args, 3));
            }
            for c in d.children.iter().take(2) {
                parts.push(c.name.clone());
            }
            if parts.is_empty() {
                "header".to_string()
            } else {
                format!("header {}", parts.join("; "))
            }
        }

        "log" => {
            if let Some(out) = d.children.iter().find(|c| c.name == "output") {
                format!("log -> {}", join_args(&out.args, 2))
            } else {
                "log".to_string()
            }
        }
        "log_skip" | "skip_log" => "log_skip".to_string(),
        "log_append" => format!("log_append {}", join_args(&d.args, 1)),

        // never echo user files/hashes
        "basic_auth" | "basicauth" => format!("basic_auth ({} users)", d.args.len()),
        "forward_auth" => format!("forward_auth -> {}", join_args(&d.args, 2)),

        "php_fastcgi" => format!("php_fastcgi -> {}", join_args(&d.args, 2)),
        "request_body" => {
            let size = d
                .children
                .iter()
                .find(|c| c.name == "max_size")
                .and_then(|c| c.args.first());
            match size {
                Some(s) => format!("request_body max_size {s}"),
                None => "request_body".to_string(),
            }
        }

        "uri" => format!("uri {}", join_args(&d.args, 3)),
        "rewrite" => format!("rewrite {}", join_args(&d.args, 3)),
        "method" => format!("method {}", join_args(&d.args, 3)),
        "try_files" => format!("try_files {}", join_args(&d.args, 4)),
        "templates" => "templates".to_string(),

        "vars" => format!("vars {}", join_args(&d.args, 2)),
        "import" => format!("import {}", join_args(&d.args, 1)),
        "push" => format!("push {}", join_args(&d.args, 1)),
        "map" => format!("map {}", join_args(&d.args, 2)),

        _ => return None,
    })
}

fn count_dirs(d: &DirNode) -> usize {
    d.children.len() + d.args.len().min(1)
}

fn parse_directives(toks: &[Token]) -> Vec<DirNode> {
    let mut out = Vec::new();
    let len = toks.len();
    let mut i = 0usize;

    while i < len {
        if toks[i].is(TokKind::Newline) || toks[i].is(TokKind::Comment) {
            i += 1;
            continue;
        }
        if toks[i].is(TokKind::CloseBrace) {
            i += 1;
            continue;
        }

        // First token of a logical line = directive name.
        if !toks[i].is_wordish() {
            i += 1;
            continue;
        }
        let name = toks[i].text.clone();
        i += 1;

        let mut args = Vec::new();
        while i < len && !toks[i].is(TokKind::Newline) {
            match toks[i].kind {
                TokKind::OpenBrace => {
                    // nested sub-block(s)
                    let mut depth = 0i32;
                    let mut children = Vec::new();
                    while i < len {
                        match toks[i].kind {
                            TokKind::OpenBrace => depth += 1,
                            TokKind::CloseBrace => {
                                depth -= 1;
                                if depth == 0 {
                                    i += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                        if toks[i].is_wordish() && depth > 0 {
                            // collect child directive line
                            let cname = toks[i].text.clone();
                            i += 1;
                            let mut cargs = Vec::new();
                            while i < len
                                && !toks[i].is(TokKind::Newline)
                                && !toks[i].is(TokKind::OpenBrace)
                                && !toks[i].is(TokKind::CloseBrace)
                            {
                                if toks[i].is_wordish() || toks[i].is(TokKind::Heredoc) {
                                    cargs.push(toks[i].text.clone());
                                }
                                i += 1;
                            }
                            children.push(DirNode {
                                name: cname,
                                args: cargs,
                                children: Vec::new(),
                            });
                            continue;
                        }
                        i += 1;
                    }
                    out.push(DirNode {
                        name,
                        args,
                        children,
                    });
                    return merge_tail(out, &toks[i..]);
                }
                TokKind::Word | TokKind::Quoted | TokKind::Heredoc => {
                    args.push(toks[i].text.clone());
                }
                _ => {}
            }
            i += 1;
        }
        out.push(DirNode {
            name,
            args,
            children: Vec::new(),
        });
    }
    out
}

fn merge_tail(mut done: Vec<DirNode>, rest: &[Token]) -> Vec<DirNode> {
    done.extend(parse_directives(rest));
    done
}

fn tls_from_directive(d: &DirNode) -> TlsInfo {
    if d.args.is_empty() {
        // block form: tls { ... }
        if let Some(dns) = d.children.iter().find(|c| c.name == "dns") {
            return TlsInfo {
                mode: TlsMode::Dns,
                detail: dns.args.first().cloned(),
            };
        }
        if d.children.iter().any(|c| c.name == "internal") {
            return TlsInfo {
                mode: TlsMode::Internal,
                detail: None,
            };
        }
        return TlsInfo {
            mode: TlsMode::Custom,
            detail: None,
        };
    }
    match d.args[0].as_str() {
        "internal" => TlsInfo {
            mode: TlsMode::Internal,
            detail: None,
        },
        other => {
            if other.contains('@') && other.contains('.') {
                TlsInfo {
                    mode: TlsMode::AcmeEmail,
                    detail: Some(other.to_string()),
                }
            } else if d.args.len() >= 2 {
                TlsInfo {
                    mode: TlsMode::Manual,
                    detail: Some(format!("{other} + {}", d.args[1])),
                }
            } else {
                TlsInfo {
                    mode: TlsMode::Custom,
                    detail: Some(other.to_string()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_proxy() {
        let info = analyze_site("app.x.com {\nreverse_proxy localhost:3000\n}\n");
        assert_eq!(info.kind, SiteKind::Proxy);
        assert_eq!(info.upstreams, vec!["localhost:3000"]);
        assert_eq!(info.tls, None);
    }

    #[test]
    fn static_with_tls_internal() {
        let info = analyze_site("web.x.com {\nroot * /srv/web\nfile_server\ntls internal\n}\n");
        assert_eq!(info.kind, SiteKind::Static);
        assert_eq!(info.tls.unwrap().mode, TlsMode::Internal);
    }

    #[test]
    fn dns_challenge_block_form() {
        let src = "x.com {\ntls {\ndns cloudflare {$CF_TOKEN}\n}\nreverse_proxy h:80\n}\n";
        let info = analyze_site(src);
        let tls = info.tls.unwrap();
        assert_eq!(tls.mode, TlsMode::Dns);
        assert_eq!(tls.detail.as_deref(), Some("cloudflare"));
    }

    #[test]
    fn unknown_directive_means_raw() {
        let info = analyze_site("x.com {\nmystery_thing foo\n}\n");
        assert_eq!(info.kind, SiteKind::Raw);
    }

    #[test]
    fn acme_email_and_manual_certs() {
        let a = analyze_site("a.com {\ntls me@co.io\n}\n");
        assert_eq!(a.tls.unwrap().mode, TlsMode::AcmeEmail);
        let b = analyze_site("b.com {\ntls /p/cert.pem /p/key.pem\n}\n");
        assert_eq!(b.tls.unwrap().mode, TlsMode::Manual);
    }

    #[test]
    fn multiple_addresses() {
        let info = analyze_site("a.com, b.com {\nrespond \"hi\"\n}\n");
        assert_eq!(info.addresses.len(), 2);
        assert_eq!(info.kind, SiteKind::Other);
    }

    #[test]
    fn heredoc_stays_simple() {
        let info = analyze_site("a.com {\nrespond <<HTML\n<h1>x</h1>\nHTML 200\n}\n");
        assert_ne!(info.kind, SiteKind::Raw);
    }

    #[test]
    fn php_site_gets_php_kind() {
        let info = analyze_site(
            "app.com {\nroot * /srv\nphp_fastcgi unix//run/php.sock\nfile_server\n}\n",
        );
        assert_eq!(info.kind, SiteKind::Php);
        assert!(info
            .details
            .iter()
            .any(|d| d.contains("php_fastcgi -> unix//run/php.sock")));
    }

    #[test]
    fn redirect_only_site() {
        let info = analyze_site("old.com {\nredir https://new.com{uri} permanent\n}\n");
        assert_eq!(info.kind, SiteKind::Other);
        assert!(info
            .details
            .iter()
            .any(|d| d == "redir https://new.com{uri} permanent"));
    }

    #[test]
    fn common_directives_produce_details_and_stay_simple() {
        let src = "x.com {\nencode zstd gzip\nheader X-Foo bar\ntry_files {path} /index.html\nuri strip_prefix /api\nmethod POST\nabort\nvars key val\nrequest_body {\nmax_size 10MB\n}\nlog {\noutput file /var/log/x.log\n}\nhandle /p* {\nrespond \"hi\"\n}\nmap {host} {dest}\n}\n";
        let info = analyze_site(src);
        assert_ne!(info.kind, SiteKind::Raw);
        for needle in [
            "encode zstd+gzip",
            "header X-Foo bar",
            "try_files {path} /index.html",
            "uri strip_prefix /api",
            "method POST",
            "abort",
            "vars key val",
            "request_body max_size 10MB",
            "log -> file /var/log/x.log",
            "map {host} {dest}",
        ] {
            assert!(
                info.details.iter().any(|d| d.contains(needle)),
                "missing detail: {needle}"
            );
        }
    }

    #[test]
    fn basic_auth_never_leaks_hashes() {
        let info = analyze_site("x.com {\nbasicauth {\nBob $2a$14$SUPERSECRETBOBCRYPT\n}\n}\n");
        assert!(!info.details.iter().any(|d| d.contains("SUPERSECRET")));
        assert!(info.details.iter().any(|d| d.contains("basic_auth (")));
    }

    #[test]
    fn details_are_capped() {
        let mut src = String::from("x.com {\n");
        for i in 0..20 {
            src.push_str(&format!("respond \"{i}\"\n"));
        }
        src.push_str("}\n");
        let info = analyze_site(&src);
        assert_eq!(info.details.len(), MAX_DETAILS + 1);
        assert!(info.details.last().unwrap().starts_with('+'));
    }
}

#[cfg(test)]
mod legacy_semantics_tests {
    use super::*;

    #[test]
    fn watch_log_flag_detected() {
        let info = analyze_site("a.com {\nimport request_watch_log\nreverse_proxy h:1\n}\n");
        assert!(info.watch_log);
        let plain = analyze_site("a.com {\nreverse_proxy h:1\n}\n");
        assert!(!plain.watch_log);
    }

    #[test]
    fn import_of_other_snippet_is_not_watch_log() {
        let info = analyze_site("a.com {\nimport common\n}\n");
        assert!(!info.watch_log);
        // unknown snippet imports stay simple (name-based known list)
        assert_ne!(info.kind, SiteKind::Raw);
    }

    #[test]
    fn reverse_proxy_option_block_marks_complex() {
        let info = analyze_site(
            "edge.com {\nreverse_proxy h2c://b:9000 {\nheader_up X-Real-IP {remote_host}\n}\n}\n",
        );
        assert!(info.complex);
        assert_eq!(info.kind, SiteKind::Raw);
        // upstreams still extracted for display
        assert_eq!(info.upstreams, vec!["h2c://b:9000"]);
    }

    #[test]
    fn cloudflare_tls_block_is_not_complex_but_custom_is() {
        let cf = analyze_site("x.com {\ntls {\ndns cloudflare tok\n}\n}\n");
        assert!(!cf.complex);
        assert_ne!(cf.kind, SiteKind::Raw);

        let duck = analyze_site("x.com {\ntls {\ndns duckdns tok\n}\n}\n");
        assert!(duck.complex);
        assert_eq!(duck.kind, SiteKind::Raw);

        let custom = analyze_site("x.com {\ntls {\nissuer acme\n}\n}\n");
        assert!(custom.complex);
    }

    #[test]
    fn nested_handle_blocks_are_not_complex() {
        let info = analyze_site("a.com {\nhandle /api* {\nreverse_proxy h:1\n}\nfile_server\n}\n");
        assert!(!info.complex);
        assert_ne!(info.kind, SiteKind::Raw);
    }
}
