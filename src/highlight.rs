// Lenient, span-aware tokenizer used only by the rustyline highlighter.
// Mirrors the boundary rules in `lexer::tokenize` but never errors: a `5.`,
// `1e`, or bare `$` is still emitted as a single span (Number or Error)
// instead of aborting, so the REPL can re-paint on every keystroke without
// flashing red while the user is still typing. The real tokenizer continues
// to enforce LRM strictness for the eval path.

use std::iter::Peekable;
use std::str::CharIndices;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TokenClass {
    Number,
    Identifier,
    SystemIdent,
    Operator,
    Punct,
    Comment,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
    pub class: TokenClass,
}

pub(crate) fn highlight_spans(input: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut chars = input.char_indices().peekable();

    while let Some(&(start, ch)) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }

        // `/` is ambiguous (comment vs. operator). Resolve before falling
        // through to the operator arm so `//` / `/*` never get folded into
        // an operator span.
        if ch == '/' {
            let mut probe = chars.clone();
            probe.next();
            match probe.peek().map(|&(_, c)| c) {
                Some('/') => {
                    chars.next();
                    chars.next();
                    let end = consume_line_comment(&mut chars, input.len());
                    out.push(Span {
                        start,
                        end,
                        class: TokenClass::Comment,
                    });
                    continue;
                }
                Some('*') => {
                    chars.next();
                    chars.next();
                    let end = consume_block_comment(&mut chars, input.len());
                    out.push(Span {
                        start,
                        end,
                        class: TokenClass::Comment,
                    });
                    continue;
                }
                _ => {}
            }
        }

        match ch {
            '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' => {
                chars.next();
                out.push(Span {
                    start,
                    end: start + ch.len_utf8(),
                    class: TokenClass::Punct,
                });
            }
            '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?'
            | ':' => {
                let end = consume_operator_run(&mut chars);
                out.push(Span {
                    start,
                    end,
                    class: TokenClass::Operator,
                });
            }
            '\'' => {
                let (end, class) = read_bare_based_literal(&mut chars);
                out.push(Span { start, end, class });
            }
            '$' => {
                let (end, class) = read_system_identifier(&mut chars);
                out.push(Span { start, end, class });
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let end = read_identifier(&mut chars);
                out.push(Span {
                    start,
                    end,
                    class: TokenClass::Identifier,
                });
            }
            c if c.is_ascii_digit() => {
                let end = read_number(&mut chars);
                out.push(Span {
                    start,
                    end,
                    class: TokenClass::Number,
                });
            }
            _ => {
                chars.next();
                out.push(Span {
                    start,
                    end: start + ch.len_utf8(),
                    class: TokenClass::Error,
                });
            }
        }
    }

    out
}

fn consume_line_comment(chars: &mut Peekable<CharIndices<'_>>, eof: usize) -> usize {
    while let Some(&(pos, c)) = chars.peek() {
        if c == '\n' {
            // Leave the newline as whitespace for the outer loop; the comment
            // span stops at the newline byte.
            return pos;
        }
        chars.next();
    }
    eof
}

fn consume_block_comment(chars: &mut Peekable<CharIndices<'_>>, eof: usize) -> usize {
    let mut prev_star = false;
    for (pos, c) in chars.by_ref() {
        if prev_star && c == '/' {
            return pos + 1;
        }
        prev_star = c == '*';
    }
    // Unterminated `/* ... ` — highlight everything as comment so the user
    // sees the run-on visually instead of a red-flashing tail.
    eof
}

// Greedy run of operator chars. `/` is intentionally excluded from the
// continuation set: a `/` mid-run could be the start of `//` or `/*`, and
// folding it into the operator span would swallow the comment.
fn consume_operator_run(chars: &mut Peekable<CharIndices<'_>>) -> usize {
    let (start, first) = chars.next().expect("operator char available");
    let mut end = start + first.len_utf8();
    if first == '/' {
        return end;
    }
    while let Some(&(pos, c)) = chars.peek() {
        if is_operator_continuation(c) {
            chars.next();
            end = pos + c.len_utf8();
        } else {
            break;
        }
    }
    end
}

fn is_operator_continuation(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?' | ':'
    )
}

fn read_identifier(chars: &mut Peekable<CharIndices<'_>>) -> usize {
    let (start, first) = chars.next().expect("identifier start char available");
    let mut end = start + first.len_utf8();
    while let Some(&(pos, c)) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            chars.next();
            end = pos + c.len_utf8();
        } else {
            break;
        }
    }
    end
}

fn read_system_identifier(chars: &mut Peekable<CharIndices<'_>>) -> (usize, TokenClass) {
    let (start, _) = chars.next().expect("`$` available");
    let mut end = start + 1;
    let mut had_body = false;
    while let Some(&(pos, c)) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            chars.next();
            end = pos + c.len_utf8();
            had_body = true;
        } else {
            break;
        }
    }
    let class = if had_body {
        TokenClass::SystemIdent
    } else {
        TokenClass::Error
    };
    (end, class)
}

// Bare based-literal form: `'b1010`, `'sh1F`, etc. A solitary `'` with no
// recognisable base char following is flagged Error so the user sees the
// mistake instead of a silent skip.
fn read_bare_based_literal(chars: &mut Peekable<CharIndices<'_>>) -> (usize, TokenClass) {
    let (start, _) = chars.next().expect("apostrophe available");
    let mut end = start + 1;
    if !matches!(chars.peek(), Some(&(_, c)) if c.is_ascii_alphabetic()) {
        return (end, TokenClass::Error);
    }
    end = consume_base_marker_and_digits(chars, end);
    (end, TokenClass::Number)
}

// Picks up the post-apostrophe tail: optional `s`/`S`, the base char, then
// the digit run (decimal/hex/octal/binary digits plus the `x`, `z`, `?`
// metalogic placeholders).
fn consume_base_marker_and_digits(
    chars: &mut Peekable<CharIndices<'_>>,
    start_end: usize,
) -> usize {
    let mut end = start_end;
    if matches!(chars.peek(), Some(&(_, 's' | 'S'))) {
        let (p, c) = chars.next().expect("`s`/`S` present");
        end = p + c.len_utf8();
    }
    if matches!(chars.peek(), Some(&(_, c)) if c.is_ascii_alphabetic()) {
        let (p, c) = chars.next().expect("base char present");
        end = p + c.len_utf8();
    }
    while let Some(&(pos, c)) = chars.peek() {
        if c.is_ascii_hexdigit()
            || c == '_'
            || c == 'x'
            || c == 'X'
            || c == 'z'
            || c == 'Z'
            || c == '?'
        {
            chars.next();
            end = pos + c.len_utf8();
        } else {
            break;
        }
    }
    end
}

// Decimal-leading numeric: digit run, optional `.digit_run`, optional
// `e[+-]?digit_run`, optional based-literal continuation after intervening
// whitespace (LRM 3.5.1 allows `8 'd6`). Lenient: a dangling `.` or `e` with
// no following digits still parses, since the user is probably mid-typing.
fn read_number(chars: &mut Peekable<CharIndices<'_>>) -> usize {
    let (start, first) = chars.next().expect("digit available");
    let mut end = start + first.len_utf8();
    end = consume_digit_run(chars, end);

    // `.` is only part of the literal when followed by another digit
    // (matches the real tokenizer's §3.5.2 rule). Otherwise leave it for
    // the outer loop to surface as Error.
    let mut probe = chars.clone();
    if matches!(probe.next(), Some((_, '.')))
        && matches!(probe.peek(), Some(&(_, c)) if c.is_ascii_digit())
    {
        let (dot_pos, _) = chars.next().expect("`.` present");
        end = dot_pos + 1;
        end = consume_digit_run(chars, end);
    }

    if matches!(chars.peek(), Some(&(_, 'e' | 'E'))) {
        let (e_pos, e_ch) = chars.next().expect("`e`/`E` present");
        end = e_pos + e_ch.len_utf8();
        if matches!(chars.peek(), Some(&(_, '+' | '-'))) {
            let (sp, sc) = chars.next().expect("sign present");
            end = sp + sc.len_utf8();
        }
        end = consume_digit_run(chars, end);
    }

    // Based-literal continuation: a (possibly ws-separated) `'<base><digits>`
    // run binds to the preceding size. Only commit the whitespace skip on
    // success so trailing spaces after a bare integer don't disappear.
    let mut probe = chars.clone();
    while matches!(probe.peek(), Some(&(_, c)) if c.is_whitespace()) {
        probe.next();
    }
    if matches!(probe.peek(), Some(&(_, '\''))) {
        *chars = probe;
        let (ap_pos, _) = chars.next().expect("apostrophe present");
        end = ap_pos + 1;
        end = consume_base_marker_and_digits(chars, end);
    }

    end
}

fn consume_digit_run(chars: &mut Peekable<CharIndices<'_>>, start_end: usize) -> usize {
    let mut end = start_end;
    while let Some(&(pos, c)) = chars.peek() {
        if c.is_ascii_digit() || c == '_' {
            chars.next();
            end = pos + c.len_utf8();
        } else {
            break;
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(input: &str) -> Vec<(&str, TokenClass)> {
        highlight_spans(input)
            .into_iter()
            .map(|s| (&input[s.start..s.end], s.class))
            .collect()
    }

    #[test]
    fn classifies_decl_and_assign() {
        assert_eq!(
            classes("reg [3:0] a = 4'b1010;"),
            vec![
                ("reg", TokenClass::Identifier),
                ("[", TokenClass::Punct),
                ("3", TokenClass::Number),
                (":", TokenClass::Operator),
                ("0", TokenClass::Number),
                ("]", TokenClass::Punct),
                ("a", TokenClass::Identifier),
                ("=", TokenClass::Operator),
                ("4'b1010", TokenClass::Number),
                (";", TokenClass::Punct),
            ]
        );
    }

    #[test]
    fn lenient_on_partial_literals() {
        // `5.` and `1e` would error in the real tokenizer; the highlighter
        // must keep coloring them as numbers so the prompt does not flash
        // red while the user is still typing.
        assert_eq!(
            classes("5."),
            vec![("5", TokenClass::Number), (".", TokenClass::Error)]
        );
        assert_eq!(classes("1e"), vec![("1e", TokenClass::Number)]);
        assert_eq!(classes("1e+"), vec![("1e+", TokenClass::Number)]);
    }

    #[test]
    fn comments_eat_to_end() {
        let spans = classes("a // tail");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0], ("a", TokenClass::Identifier));
        assert_eq!(spans[1], ("// tail", TokenClass::Comment));

        let spans = classes("/* unterminated");
        assert_eq!(spans, vec![("/* unterminated", TokenClass::Comment)]);
    }

    #[test]
    fn bare_dollar_is_error_but_system_call_is_not() {
        assert_eq!(classes("$"), vec![("$", TokenClass::Error)]);
        assert_eq!(
            classes("$finish"),
            vec![("$finish", TokenClass::SystemIdent)]
        );
    }

    #[test]
    fn slash_does_not_swallow_following_comment() {
        // `+//x` must parse as Operator `+` then Comment `//x`. If `/` were in
        // the operator-continuation set, the `//` would get eaten as part of
        // the operator span and the comment colouring would silently break.
        assert_eq!(
            classes("+//x"),
            vec![("+", TokenClass::Operator), ("//x", TokenClass::Comment)]
        );
    }
}
