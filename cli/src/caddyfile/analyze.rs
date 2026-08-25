//! Structural analysis of a single site block: which directives it uses,
//! whether we can render a "simple" structured summary, upstreams, TLS mode.

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
    "skip_log",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteKind {
    Proxy,
    Static,
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

    let kind = if !is_simple {
        SiteKind::Raw
    } else if dirs.iter().any(|d| d.name == "reverse_proxy") {
        SiteKind::Proxy
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

    SiteInfo {
        addresses,
        kind,
        upstreams,
        tls,
        directives,
    }
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
                    detail: Some(format!("{} + {}", other, d.args[1])),
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
}
