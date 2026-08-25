//! Tokenizer for the Caddyfile format.
//!
//! The lexer preserves byte spans for every token so that site blocks can be
//! split out of a monolithic Caddyfile *verbatim*, without reformatting.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokKind {
    Word,
    Quoted,
    Heredoc,
    OpenBrace,
    CloseBrace,
    Newline,
    Comment,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokKind,
    /// Decoded value (quotes stripped, escapes resolved) for
    /// Word/Quoted/Heredoc; raw text otherwise.
    pub text: String,
    pub span: Range<usize>,
}

impl Token {
    pub fn is(&self, kind: TokKind) -> bool {
        self.kind == kind
    }

    /// `true` when this token can act as an address / directive name.
    pub fn is_wordish(&self) -> bool {
        matches!(self.kind, TokKind::Word | TokKind::Quoted)
    }
}

pub fn tokenize(src: &str) -> Vec<Token> {
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut toks = Vec::new();
    let mut i = 0usize;

    while i < len {
        let b = bytes[i];
        match b {
            b'\n' => {
                toks.push(Token {
                    kind: TokKind::Newline,
                    text: "\n".into(),
                    span: i..i + 1,
                });
                i += 1;
            }
            b' ' | b'\t' | b'\r' => {
                i += 1;
            }
            b'#' => {
                let start = i;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                toks.push(Token {
                    kind: TokKind::Comment,
                    text: src[start..i].to_string(),
                    span: start..i,
                });
            }
            b'"' => {
                let start = i;
                i += 1;
                let mut value = String::new();
                while i < len {
                    match bytes[i] {
                        b'\\' if i + 1 < len => {
                            value.push(unescape_char(bytes[i + 1]));
                            i += 2;
                        }
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => {
                            // advance one UTF-8 scalar
                            let ch_len = utf8_len(bytes[i]);
                            value.push_str(&src[i..(i + ch_len).min(len)]);
                            i += ch_len;
                        }
                    }
                }
                toks.push(Token {
                    kind: TokKind::Quoted,
                    text: value,
                    span: start..i,
                });
            }
            b'`' => {
                let start = i;
                i += 1;
                let mut value = String::new();
                while i < len && bytes[i] != b'`' {
                    let ch_len = utf8_len(bytes[i]);
                    value.push_str(&src[i..(i + ch_len).min(len)]);
                    i += ch_len;
                }
                i = (i + 1).min(len); // consume closing backtick
                toks.push(Token {
                    kind: TokKind::Quoted,
                    text: value,
                    span: start..i,
                });
            }
            b'{' if standalone(bytes, i) => {
                toks.push(Token {
                    kind: TokKind::OpenBrace,
                    text: "{".into(),
                    span: i..i + 1,
                });
                i += 1;
            }
            b'}' if standalone(bytes, i) => {
                toks.push(Token {
                    kind: TokKind::CloseBrace,
                    text: "}".into(),
                    span: i..i + 1,
                });
                i += 1;
            }
            b'<' if i + 1 < len && bytes[i + 1] == b'<' => {
                if let Some(tok) = lex_heredoc(src, bytes, &mut i) {
                    toks.push(tok);
                } else {
                    // Not a valid heredoc opener; fall through as a word.
                    let tok = lex_word(src, bytes, &mut i);
                    toks.push(tok);
                }
            }
            _ => {
                let tok = lex_word(src, bytes, &mut i);
                toks.push(tok);
            }
        }
    }

    toks
}

/// `{` / `}` only count as block delimiters when surrounded by whitespace.
fn standalone(bytes: &[u8], i: usize) -> bool {
    let prev_ws = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\r' | b'\n');
    let next_ws = i + 1 >= bytes.len() || matches!(bytes[i + 1], b' ' | b'\t' | b'\r' | b'\n');
    prev_ws && next_ws
}

fn lex_word(src: &str, bytes: &[u8], i: &mut usize) -> Token {
    let start = *i;
    let len = bytes.len();
    while *i < len {
        let b = bytes[*i];
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            break;
        }
        // A brace that would be "standalone" terminates the word.
        if matches!(b, b'{' | b'}') && standalone(bytes, *i) {
            break;
        }
        *i += utf8_len(b);
    }
    let text = src[start..*i].to_string();
    Token {
        kind: TokKind::Word,
        text,
        span: start..*i,
    }
}

fn heredoc_marker(bytes: &[u8], mut i: usize) -> Option<String> {
    // i points at the first '<'
    i += 2;
    let start = i;
    let len = bytes.len();
    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    if i == start {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[start..i]).into_owned())
}

fn lex_heredoc(src: &str, bytes: &[u8], i: &mut usize) -> Option<Token> {
    let start = *i;
    let marker = heredoc_marker(bytes, *i)?;
    let len = bytes.len();

    // After the marker there may be trailing spaces then a newline.
    let mut j = start + 2 + marker.len();
    while j < len && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\r') {
        j += 1;
    }
    if j >= len || bytes[j] != b'\n' {
        return None; // not a heredoc after all
    }
    j += 1;

    let mut content_lines: Vec<&str> = Vec::new();
    loop {
        if j >= len {
            // Unterminated heredoc: swallow the rest of the file.
            break;
        }
        let line_start = j;
        while j < len && bytes[j] != b'\n' {
            j += 1;
        }
        let mut line_end = j.min(len);
        if line_end > line_start && bytes[line_end - 1] == b'\r' {
            line_end -= 1;
        }
        let line = &src[line_start..line_end];
        if line.trim() == marker {
            if j < len {
                j += 1; // consume newline of closing marker line
            }
            *i = j;
            return Some(Token {
                kind: TokKind::Heredoc,
                text: content_lines.join("\n"),
                span: start..j,
            });
        }
        content_lines.push(line);
        if j < len {
            j += 1; // consume newline
        }
    }

    *i = len;
    Some(Token {
        kind: TokKind::Heredoc,
        text: content_lines.join("\n"),
        span: start..len,
    })
}

fn unescape_char(b: u8) -> char {
    match b {
        b'n' => '\n',
        b't' => '\t',
        b'r' => '\r',
        other => other as char,
    }
}

fn utf8_len(first_byte: u8) -> usize {
    match first_byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokKind> {
        tokenize(src).into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn basic_tokens() {
        assert_eq!(
            kinds("example.com {\n reverse_proxy localhost:3000\n}\n"),
            vec![
                TokKind::Word,
                TokKind::OpenBrace,
                TokKind::Newline,
                TokKind::Word,
                TokKind::Word,
                TokKind::Newline,
                TokKind::CloseBrace,
                TokKind::Newline,
            ]
        );
    }

    #[test]
    fn quoted_string() {
        let toks = tokenize(r#"respond "hello \"world\"""#);
        assert_eq!(toks[0].text, "respond");
        assert_eq!(toks[1].text, r#"hello "world""#);
        assert_eq!(toks[1].kind, TokKind::Quoted);
    }

    #[test]
    fn placeholder_braces_are_words() {
        let toks = tokenize("rewrite {http.request.uri} /index.php");
        assert_eq!(toks.len(), 3);
        assert_eq!(toks[1].text, "{http.request.uri}");
    }

    #[test]
    fn env_placeholder_in_word() {
        let toks = tokenize("tls {$TLS_EMAIL}");
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[1].text, "{$TLS_EMAIL}");
    }

    #[test]
    fn comment_runs_to_eol() {
        let toks = tokenize("# full line\nhost # trailing");
        assert_eq!(
            kinds("# x\ny"),
            vec![TokKind::Comment, TokKind::Newline, TokKind::Word]
        );
        assert_eq!(toks.last().unwrap().text, "# trailing");
    }

    #[test]
    fn heredoc_basic() {
        let src = "respond <<HTML\n  <h1>hi</h1>\n  HTML\n";
        let toks = tokenize(src);
        // respond + heredoc (closing-marker line and its newline consumed)
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[1].kind, TokKind::Heredoc);
        assert_eq!(toks[1].text, "  <h1>hi</h1>");
        assert_eq!(toks[1].span.end, src.len());
    }

    #[test]
    fn double_lt_without_marker_is_word() {
        let toks = tokenize("a <<b c");
        assert_eq!(toks[1].text, "<<b");
    }

    #[test]
    fn spans_roundtrip() {
        let src = "a.com {\nroot * /srv\n}\nb.com {\n}\n";
        let toks = tokenize(src);
        let rebuilt: String = toks
            .iter()
            .map(|t| &src[t.span.start..t.span.end])
            .collect();
        // every non-whitespace byte is covered by some span, newlines included
        assert_eq!(rebuilt, "a.com{\nroot*/srv\n}\nb.com{\n}\n");
    }
}
