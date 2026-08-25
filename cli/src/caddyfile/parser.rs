//! Top-level document parsing: splits a Caddyfile into snippets, the global
//! options block and individual site blocks — all while keeping byte spans so
//! blocks can be extracted verbatim.

use super::lexer::{tokenize, TokKind, Token};
use std::ops::Range;

#[derive(Debug, Clone)]
pub struct SiteBlock {
    /// Raw address tokens as written (e.g. `example.com`, `:8080`, `*.foo.dev`).
    pub addresses: Vec<String>,
    /// Header line tokens including the opening brace.
    #[allow(dead_code)]
    pub header_tokens: Vec<Token>,
    /// Byte span covering the header line through the closing brace
    /// (plus immediately trailing newlines).
    pub full_span: Range<usize>,
}

impl SiteBlock {
    pub fn primary_address(&self) -> Option<&str> {
        self.addresses.first().map(|s| s.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Snippet {
    #[allow(dead_code)]
    pub name: String,
    pub full_span: Range<usize>,
}

#[derive(Debug, Clone)]
pub enum TopLevel {
    Site(SiteBlock),
    Snippet(Snippet),
    /// Global options block (`{ ... }` with no addresses) or anything else we
    /// don't interpret (imports, stray comments...). Kept verbatim.
    Other {
        span: Range<usize>,
        #[allow(dead_code)] // consumed by tests + future `init` refinements
        is_global: bool,
    },
}

#[derive(Debug)]
pub struct Document {
    pub src: String,
    pub items: Vec<TopLevel>,
}

impl Document {
    pub fn parse(src: &str) -> Document {
        let toks = tokenize(src);
        let len = toks.len();
        let mut items = Vec::new();
        let mut i = 0usize;

        while i < len {
            // Skip blank lines between top-level items.
            if toks[i].is(TokKind::Newline) {
                i += 1;
                continue;
            }

            // Collect one logical line of tokens [line_start..line_end).
            let line_start = i;
            while i < len && !toks[i].is(TokKind::Newline) {
                i += 1;
            }
            let line_end = i;
            let line = &toks[line_start..line_end];
            if line.is_empty() {
                continue;
            }

            // Global options block: a line that is exactly `{`.
            if line.len() == 1 && line[0].is(TokKind::OpenBrace) {
                let after = consume_block(&toks, line_start);
                let (next_i, byte_end) = swallow_trailing_newlines(&toks, after);
                items.push(TopLevel::Other {
                    span: line[0].span.start..byte_end,
                    is_global: true,
                });
                i = next_i;
                continue;
            }

            // Snippet header: `(name)` followed by `{` on the same line.
            if line.len() >= 2
                && line[0].is_wordish()
                && line[0].text.starts_with('(')
                && line[0].text.ends_with(')')
                && line[line.len() - 1].is(TokKind::OpenBrace)
            {
                let name = line[0].text[1..line[0].text.len() - 1].to_string();
                let after = consume_block(&toks, line_end - 1);
                let (next_i, byte_end) = swallow_trailing_newlines(&toks, after);
                items.push(TopLevel::Snippet(Snippet {
                    name,
                    full_span: line[0].span.start..byte_end,
                }));
                i = next_i;
                continue;
            }

            // Site block: address tokens then `{` as the last token of the line.
            if line.len() >= 2 && line[line.len() - 1].is(TokKind::OpenBrace) {
                // `a.com, b.com {` leaves the comma glued to the first token.
                let clean = |raw: &str| {
                    raw.trim_matches(|c| c == ',' || c == ';')
                        .trim()
                        .to_string()
                };
                let addresses: Vec<String> = line[..line.len() - 1]
                    .iter()
                    .filter(|t| t.is_wordish())
                    .map(|t| clean(&t.text))
                    .filter(|s| !s.is_empty())
                    .collect();
                let after = consume_block(&toks, line_end - 1);
                let (next_i, byte_end) = swallow_trailing_newlines(&toks, after);
                items.push(TopLevel::Site(SiteBlock {
                    addresses,
                    header_tokens: line.to_vec(),
                    full_span: line[0].span.start..byte_end,
                }));
                i = next_i;
                continue;
            }

            // Anything else (e.g. `import foo`) — keep verbatim, one line.
            let end = line[line.len() - 1].span.end;
            items.push(TopLevel::Other {
                span: line[0].span.start..end,
                is_global: false,
            });
            if i < len {
                i += 1; // consume terminating Newline
            }
        }

        Document {
            src: src.to_string(),
            items,
        }
    }

    pub fn sites(&self) -> Vec<&SiteBlock> {
        self.items
            .iter()
            .filter_map(|it| match it {
                TopLevel::Site(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    #[cfg(test)]
    pub fn global_span(&self) -> Option<Range<usize>> {
        self.items.iter().find_map(|it| match it {
            TopLevel::Other {
                span,
                is_global: true,
            } => Some(span.clone()),
            _ => None,
        })
    }

    /// Byte slice of a site block, trimmed of trailing newlines but guaranteed
    /// to end with exactly one `\n`.
    pub fn site_text(&self, site: &SiteBlock) -> String {
        let mut text = self.src[site.full_span.clone()].trim_end().to_string();
        text.push('\n');
        text
    }
}

/// Starting at `open_idx` (an OpenBrace), return the token index just past the
/// matching CloseBrace. Unbalanced input consumes to EOF.
fn consume_block(toks: &[Token], open_idx: usize) -> usize {
    let mut depth = 0i32;
    let mut j = open_idx;
    while j < toks.len() {
        match toks[j].kind {
            TokKind::OpenBrace => depth += 1,
            TokKind::CloseBrace => {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            _ => {}
        }
        j += 1;
    }
    toks.len()
}

/// Advance past any Newline tokens following a block; returns the new token
/// index and the byte offset just past the last consumed token.
fn swallow_trailing_newlines(toks: &[Token], mut idx: usize) -> (usize, usize) {
    while idx < toks.len() && toks[idx].is(TokKind::Newline) {
        idx += 1;
    }
    let byte_end = if idx > 0 { toks[idx - 1].span.end } else { 0 };
    (idx, byte_end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_two_sites() {
        let src = "a.com {\nrespond \"A\"\n}\n\nb.com {\nrespond \"B\"\n}\n";
        let doc = Document::parse(src);
        let sites = doc.sites();
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].addresses, vec!["a.com"]);
        assert_eq!(doc.site_text(sites[0]), "a.com {\nrespond \"A\"\n}\n");
        assert_eq!(doc.site_text(sites[1]), "b.com {\nrespond \"B\"\n}\n");
    }

    #[test]
    fn verbatim_bytes_preserved() {
        let src = "a.com\t{\n\trespond  \"A\"   # spaced\n}\n";
        let doc = Document::parse(src);
        let text = doc.site_text(doc.sites()[0]);
        assert!(text.contains("a.com\t{"));
        assert!(text.contains("\"A\"   # spaced"));
    }

    #[test]
    fn snippet_and_global_preserved() {
        let src =
            "{\n\temail me@x.com\n}\n(common) {\n\tencode gzip\n}\na.com {\nimport common\n}\n";
        let doc = Document::parse(src);
        assert!(doc.global_span().is_some());
        let snips: Vec<_> = doc
            .items
            .iter()
            .filter_map(|i| match i {
                TopLevel::Snippet(s) => Some(s.name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(snips, vec!["common"]);
        assert_eq!(doc.sites().len(), 1);
    }

    #[test]
    fn multiline_addresses_and_quotes() {
        let src = "\"a.com\" b.com:8443 {\nreverse_proxy localhost:8080 localhost:8081\n}\n";
        let doc = Document::parse(src);
        let sites = doc.sites();
        assert_eq!(sites[0].addresses, vec!["a.com", "b.com:8443"]);
    }

    #[test]
    fn nested_braces_in_directives_do_not_confuse() {
        let src = "a.com {\nhandle /api* {\nreverse_proxy h:1\n}\nfile_server\n}\nb.com {\n}\n";
        let doc = Document::parse(src);
        let sites = doc.sites();
        assert_eq!(sites.len(), 2);
        let t = doc.site_text(sites[0]);
        assert!(t.contains("handle /api*"));
        assert!(t.ends_with("}\n"));
    }

    #[test]
    fn placeholder_keeps_site_split_intact() {
        let src = "a.com {\nrewrite {http.request.uri} /x\n}\nb.com {\n}\n";
        let doc = Document::parse(src);
        assert_eq!(doc.sites().len(), 2);
    }

    #[test]
    fn import_lines_survive_as_other() {
        let src = "import /etc/caddy/snippets/*.caddy\na.com {\n}\n";
        let doc = Document::parse(src);
        let others: Vec<_> = doc
            .items
            .iter()
            .filter_map(|i| match i {
                TopLevel::Other { span, .. } => Some(span.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(others.len(), 1);
        assert!(src[others[0].clone()].starts_with("import"));
        assert_eq!(doc.sites().len(), 1);
    }
}
