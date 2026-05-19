#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Token {
    IntegerLiteral(String),
    // `$identifier` — system task or function name. Per LRM A.9.3 the name
    // matches `$[a-zA-Z0-9_$]+`; the `$` shall not be followed by white space
    // (LRM 19.5 / README "Identifier white spaces").
    SystemIdentifier(String),
    LParen,
    RParen,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Power,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    EqualEqual,
    NotEqual,
    CaseEqual,
    CaseNotEqual,
    Bang,
    LogicalAnd,
    LogicalOr,
    Tilde,
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    BitwiseXnor,
    BitwiseNand,
    BitwiseNor,
    LogicalShiftLeft,
    LogicalShiftRight,
    ArithmeticShiftLeft,
    ArithmeticShiftRight,
    Question,
    Colon,
    LBrace,
    RBrace,
    Comma,
}

pub(crate) fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();

    while let Some((_, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }

        match ch {
            '(' => tokens.push(Token::LParen),
            ')' => tokens.push(Token::RParen),
            '+' => tokens.push(Token::Plus),
            '-' => tokens.push(Token::Minus),
            '/' => tokens.push(Token::Slash),
            '%' => tokens.push(Token::Percent),
            '*' => {
                if matches!(chars.peek(), Some((_, '*'))) {
                    chars.next();
                    tokens.push(Token::Power);
                } else {
                    tokens.push(Token::Star);
                }
            }
            '<' => {
                // Greedy: `<<<` (arithmetic left shift) > `<<` (logical left
                // shift) > `<=` > `<`. Longest-prefix wins, mirroring how the
                // existing `==`/`===` and `~^`/`~&` paths disambiguate.
                if matches!(chars.peek(), Some((_, '<'))) {
                    chars.next();
                    if matches!(chars.peek(), Some((_, '<'))) {
                        chars.next();
                        tokens.push(Token::ArithmeticShiftLeft);
                    } else {
                        tokens.push(Token::LogicalShiftLeft);
                    }
                } else if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    tokens.push(Token::LessEqual);
                } else {
                    tokens.push(Token::Less);
                }
            }
            '>' => {
                if matches!(chars.peek(), Some((_, '>'))) {
                    chars.next();
                    if matches!(chars.peek(), Some((_, '>'))) {
                        chars.next();
                        tokens.push(Token::ArithmeticShiftRight);
                    } else {
                        tokens.push(Token::LogicalShiftRight);
                    }
                } else if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    tokens.push(Token::GreaterEqual);
                } else {
                    tokens.push(Token::Greater);
                }
            }
            '=' => {
                if !matches!(chars.peek(), Some((_, '='))) {
                    return Err("expected `==` or `===`".to_string());
                }
                chars.next();
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    tokens.push(Token::CaseEqual);
                } else {
                    tokens.push(Token::EqualEqual);
                }
            }
            '!' => {
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    if matches!(chars.peek(), Some((_, '='))) {
                        chars.next();
                        tokens.push(Token::CaseNotEqual);
                    } else {
                        tokens.push(Token::NotEqual);
                    }
                } else {
                    tokens.push(Token::Bang);
                }
            }
            '&' => {
                if matches!(chars.peek(), Some((_, '&'))) {
                    chars.next();
                    tokens.push(Token::LogicalAnd);
                } else {
                    tokens.push(Token::BitwiseAnd);
                }
            }
            '|' => {
                if matches!(chars.peek(), Some((_, '|'))) {
                    chars.next();
                    tokens.push(Token::LogicalOr);
                } else {
                    tokens.push(Token::BitwiseOr);
                }
            }
            '^' => {
                // ^~ is the alternate spelling of the bitwise equivalence
                // operator ~^ (LRM 5.1.10). Lex the two-char form greedily so
                // both spellings collapse onto the same token.
                if matches!(chars.peek(), Some((_, '~'))) {
                    chars.next();
                    tokens.push(Token::BitwiseXnor);
                } else {
                    tokens.push(Token::BitwiseXor);
                }
            }
            '~' => {
                // ~^ is the bitwise equivalence operator (LRM 5.1.10); ~& and
                // ~| are the unary-only NAND/NOR reduction operators
                // (LRM 5.1.11 + A.8.6). All three are lexed greedily so a
                // bare `~` only appears in a position where it must be the
                // per-bit unary NOT.
                match chars.peek() {
                    Some((_, '^')) => {
                        chars.next();
                        tokens.push(Token::BitwiseXnor);
                    }
                    Some((_, '&')) => {
                        chars.next();
                        tokens.push(Token::BitwiseNand);
                    }
                    Some((_, '|')) => {
                        chars.next();
                        tokens.push(Token::BitwiseNor);
                    }
                    _ => tokens.push(Token::Tilde),
                }
            }
            '?' => tokens.push(Token::Question),
            ':' => tokens.push(Token::Colon),
            '{' => tokens.push(Token::LBrace),
            '}' => tokens.push(Token::RBrace),
            ',' => tokens.push(Token::Comma),
            '\'' => {
                tokens.push(Token::IntegerLiteral(read_based_literal_after_apostrophe(
                    &mut chars,
                )?));
            }
            '$' => {
                tokens.push(Token::SystemIdentifier(read_system_identifier(&mut chars)?));
            }
            _ => {
                tokens.push(Token::IntegerLiteral(read_integer_literal(ch, &mut chars)?));
            }
        }
    }

    Ok(tokens)
}

fn read_integer_literal<I>(
    first_ch: char,
    chars: &mut std::iter::Peekable<I>,
) -> Result<String, String>
where
    I: Iterator<Item = (usize, char)> + Clone,
{
    let mut literal = String::new();
    literal.push(first_ch);

    while let Some((_, next_ch)) = chars.peek().copied() {
        if next_ch.is_whitespace() || is_expression_delimiter(next_ch) || next_ch == '\'' {
            break;
        }

        chars.next();
        literal.push(next_ch);
    }

    let mut cursor = chars.clone();
    skip_whitespace(&mut cursor);

    if matches!(cursor.peek(), Some((_, '\''))) {
        *chars = cursor;
        chars.next();
        literal.push('\'');

        let (_, base_ch) = chars
            .next()
            .ok_or_else(|| "missing base after apostrophe".to_string())?;
        if base_ch.is_whitespace() {
            return Err("missing base after apostrophe".to_string());
        }
        literal.push(base_ch);

        if matches!(base_ch, 's' | 'S') {
            let (_, signed_base_ch) = chars
                .next()
                .ok_or_else(|| "missing base after signed marker".to_string())?;
            if signed_base_ch.is_whitespace() {
                return Err("missing base after signed marker".to_string());
            }
            literal.push(signed_base_ch);
        }

        let mut saw_digit = false;
        while let Some((_, next_ch)) = chars.peek().copied() {
            if is_expression_delimiter(next_ch) {
                break;
            }

            // Whitespace before the first digit is OK (e.g. `8'd 6`); once
            // we've started reading digits it terminates the literal so a
            // following `?` (or any other char) tokenises separately.
            if next_ch.is_whitespace() {
                if saw_digit {
                    break;
                }
                chars.next();
                continue;
            }

            chars.next();
            literal.push(next_ch);
            saw_digit = true;
        }

        if !saw_digit {
            return Err("missing digits in integer literal".to_string());
        }
    }

    Ok(literal)
}

fn read_based_literal_after_apostrophe<I>(
    chars: &mut std::iter::Peekable<I>,
) -> Result<String, String>
where
    I: Iterator<Item = (usize, char)> + Clone,
{
    let mut literal = String::from("'");

    let (_, base_ch) = chars
        .next()
        .ok_or_else(|| "missing base after apostrophe".to_string())?;
    if base_ch.is_whitespace() {
        return Err("missing base after apostrophe".to_string());
    }
    literal.push(base_ch);

    if matches!(base_ch, 's' | 'S') {
        let (_, signed_base_ch) = chars
            .next()
            .ok_or_else(|| "missing base after signed marker".to_string())?;
        if signed_base_ch.is_whitespace() {
            return Err("missing base after signed marker".to_string());
        }
        literal.push(signed_base_ch);
    }

    let mut saw_digit = false;
    while let Some((_, next_ch)) = chars.peek().copied() {
        if is_expression_delimiter(next_ch) {
            break;
        }

        if next_ch.is_whitespace() {
            if saw_digit {
                break;
            }
            chars.next();
            continue;
        }

        chars.next();
        literal.push(next_ch);
        saw_digit = true;
    }

    if !saw_digit {
        return Err("missing digits in integer literal".to_string());
    }

    Ok(literal)
}

fn skip_whitespace<I>(chars: &mut std::iter::Peekable<I>)
where
    I: Iterator<Item = (usize, char)>,
{
    while matches!(chars.peek(), Some((_, ch)) if ch.is_whitespace()) {
        chars.next();
    }
}

// LRM A.9.3: `$[a-zA-Z0-9_$]+`. The leading `$` is already consumed; at least
// one identifier character must follow, and per LRM 19.5 / README "Identifier
// white spaces" the `$` shall not be followed by whitespace, so a bare `$` or
// `$ name` is a lex error rather than silently accepting it.
fn read_system_identifier<I>(chars: &mut std::iter::Peekable<I>) -> Result<String, String>
where
    I: Iterator<Item = (usize, char)>,
{
    let mut name = String::from("$");
    while let Some((_, ch)) = chars.peek().copied() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
            chars.next();
            name.push(ch);
        } else {
            break;
        }
    }

    if name.len() == 1 {
        return Err("missing identifier after `$`".to_string());
    }

    Ok(name)
}

fn is_expression_delimiter(ch: char) -> bool {
    // Note: `?` is intentionally NOT a delimiter even though it tokenises
    // as the conditional operator's `?` — inside a based literal it is the
    // alias for `z` (LRM 3.5), and `read_integer_literal`'s pre-apostrophe
    // loop already exits on any non-digit, so `1?2` still tokenises as
    // `1`, `?`, `2`.
    matches!(
        ch,
        '(' | ')'
            | '+'
            | '-'
            | '*'
            | '/'
            | '%'
            | '<'
            | '>'
            | '='
            | '!'
            | '&'
            | '|'
            | '^'
            | '~'
            | ':'
            | '{'
            | '}'
            | ','
            | '$'
    )
}
