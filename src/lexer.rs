#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Token {
    IntegerLiteral(String),
    // LRM §3.5.2 / A.8.7. Two forms are accepted, both required to have at
    // least one digit on each side of the decimal point:
    //   unsigned_number . unsigned_number
    //   unsigned_number [. unsigned_number] [eE] [+|-] unsigned_number
    // Underscores are legal inside any digit run and are stripped at parse
    // time. The string here is the raw lexeme (still containing `_` and the
    // original `e`/`E` casing); f64 conversion happens in the parser.
    RealLiteral(String),
    // `$identifier` — system task or function name. Per LRM A.9.3 the name
    // matches `$[a-zA-Z0-9_$]+`; the `$` shall not be followed by white space
    // (LRM 19.5 / README "Identifier white spaces").
    SystemIdentifier(String),
    // LRM A.9.3 / 3.7.1: `simple_identifier ::= [a-zA-Z_]{[a-zA-Z0-9_$]}`.
    // Keywords (e.g. `reg`, `signed`) are not lexed separately; the parser
    // matches them on the identifier string.
    Identifier(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Assign,
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
    Semicolon,
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
                // Greedy: `===` > `==` > `=` (blocking assignment, LRM A.6.2).
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    if matches!(chars.peek(), Some((_, '='))) {
                        chars.next();
                        tokens.push(Token::CaseEqual);
                    } else {
                        tokens.push(Token::EqualEqual);
                    }
                } else {
                    tokens.push(Token::Assign);
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
            '[' => tokens.push(Token::LBracket),
            ']' => tokens.push(Token::RBracket),
            ',' => tokens.push(Token::Comma),
            ';' => tokens.push(Token::Semicolon),
            '\'' => {
                tokens.push(Token::IntegerLiteral(read_based_literal_after_apostrophe(
                    &mut chars,
                )?));
            }
            '$' => {
                tokens.push(Token::SystemIdentifier(read_system_identifier(&mut chars)?));
            }
            _ => {
                // Simple identifiers (LRM 3.7.1) start with a letter or
                // underscore and continue with [a-zA-Z0-9_$]. Route them here
                // before the integer/real reader, otherwise a leading letter
                // would land in the literal path and surface as "invalid
                // decimal digits".
                if ch.is_ascii_alphabetic() || ch == '_' {
                    tokens.push(read_simple_identifier(ch, &mut chars));
                } else {
                    tokens.push(read_integer_or_real_literal(ch, &mut chars)?);
                }
            }
        }
    }

    Ok(tokens)
}

// Decides between Token::IntegerLiteral, Token::RealLiteral, and the based
// integer form (which always lexes as IntegerLiteral). LRM §3.5.2 reals
// require a digit on each side of the decimal point, so we only treat `.`
// as part of the literal when both the preceding char is a digit and the
// next char is a digit. An exponent (`e` or `E`) followed by an optional
// sign and at least one digit is also accepted as the second real form.
// If neither real-form continuation is found, fall back to the existing
// integer / based-integer path.
fn read_integer_or_real_literal<I>(
    first_ch: char,
    chars: &mut std::iter::Peekable<I>,
) -> Result<Token, String>
where
    I: Iterator<Item = (usize, char)> + Clone,
{
    // Non-digit starts cannot begin a real (LRM §3.5.2 requires digits on
    // both sides of the decimal point and at the start of the exponent
    // form). Defer to the permissive integer reader so the existing error
    // path — token reaches parse_integer and fails there — is preserved.
    if !first_ch.is_ascii_digit() {
        return Ok(Token::IntegerLiteral(read_integer_literal_from_digits(
            String::from(first_ch),
            chars,
        )?));
    }

    let mut digits = String::new();
    digits.push(first_ch);
    while let Some((_, next_ch)) = chars.peek().copied() {
        if next_ch.is_ascii_digit() || next_ch == '_' {
            chars.next();
            digits.push(next_ch);
        } else {
            break;
        }
    }

    // Real candidate: '.' immediately followed by a digit. We must NOT
    // consume the '.' if the next char is not a digit (e.g. `5.` is an
    // illegal real per §3.5.2; treating it as `5` followed by `.` lets the
    // outer tokenizer surface the `.` as an unexpected character).
    let real_after_dot = {
        let mut lookahead = chars.clone();
        matches!(lookahead.next(), Some((_, '.')))
            && matches!(lookahead.peek(), Some((_, ch)) if ch.is_ascii_digit())
    };

    // Real candidate: a digit run followed by `e` / `E` is always a real
    // attempt — there is no other token shape that starts that way (the
    // based-integer path is gated on `'`, not `e`). We only check for the
    // letter here; `consume_exponent` enforces the LRM A.8.7 digit-leading
    // requirement on whatever follows, which lets us surface a precise
    // "missing exponent digits in real literal" error for malformed forms
    // like `1e_3` or `1e` instead of falling through to the integer path
    // and producing a misleading "invalid decimal digits" message.
    let real_after_exp = {
        let mut lookahead = chars.clone();
        matches!(lookahead.next(), Some((_, 'e' | 'E')))
    };

    if real_after_dot {
        chars.next();
        digits.push('.');
        while let Some((_, next_ch)) = chars.peek().copied() {
            if next_ch.is_ascii_digit() || next_ch == '_' {
                chars.next();
                digits.push(next_ch);
            } else {
                break;
            }
        }
        // Optional exponent after the fractional part.
        if let Some((_, exp_ch)) = chars.peek().copied()
            && (exp_ch == 'e' || exp_ch == 'E')
        {
            consume_exponent(&mut digits, chars)?;
        }
        return Ok(Token::RealLiteral(digits));
    }

    if real_after_exp {
        consume_exponent(&mut digits, chars)?;
        return Ok(Token::RealLiteral(digits));
    }

    // No real continuation: fall through to the existing integer / based
    // integer reader. We've already buffered the leading digit run, so
    // delegate the post-digit logic (whitespace, apostrophe pickup) to the
    // helper.
    Ok(Token::IntegerLiteral(read_integer_literal_from_digits(
        digits, chars,
    )?))
}

fn consume_exponent<I>(
    digits: &mut String,
    chars: &mut std::iter::Peekable<I>,
) -> Result<(), String>
where
    I: Iterator<Item = (usize, char)>,
{
    let (_, exp_ch) = chars
        .next()
        .ok_or_else(|| "missing exponent in real literal".to_string())?;
    digits.push(exp_ch);
    if matches!(chars.peek(), Some((_, '+' | '-'))) {
        let (_, sign_ch) = chars.next().expect("guarded by peek");
        digits.push(sign_ch);
    }

    // LRM A.8.7: `unsigned_number ::= decimal_digit { _ | decimal_digit }`.
    // The exponent's digit run must *start* with a decimal digit — a
    // leading underscore (e.g. `5.0e_3`, `1e+_3`) is not a legal
    // unsigned_number. Enforce digit-leading here; the trailing run can
    // then freely mix digits and underscores.
    match chars.peek().copied() {
        Some((_, ch)) if ch.is_ascii_digit() => {
            chars.next();
            digits.push(ch);
        }
        _ => return Err("missing exponent digits in real literal".to_string()),
    }
    while let Some((_, next_ch)) = chars.peek().copied() {
        if next_ch.is_ascii_digit() || next_ch == '_' {
            chars.next();
            digits.push(next_ch);
        } else {
            break;
        }
    }
    Ok(())
}

fn read_integer_literal_from_digits<I>(
    initial: String,
    chars: &mut std::iter::Peekable<I>,
) -> Result<String, String>
where
    I: Iterator<Item = (usize, char)> + Clone,
{
    let mut literal = initial;

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
        read_base_marker_and_digits(&mut literal, chars)?;
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
    read_base_marker_and_digits(&mut literal, chars)?;
    Ok(literal)
}

// Shared tail for both `<size>'<base><digits>` and bare `'<base><digits>`
// based-literal forms. Caller has already consumed the apostrophe and pushed
// it onto `literal`; this reads the base char (with an optional `s`/`S`
// signed marker) and then the digit run.
fn read_base_marker_and_digits<I>(
    literal: &mut String,
    chars: &mut std::iter::Peekable<I>,
) -> Result<(), String>
where
    I: Iterator<Item = (usize, char)>,
{
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
        // `?` is a valid based-literal digit (alias for `z`, LRM 3.5), so
        // it must not terminate the post-apostrophe digit run even though
        // it is an expression delimiter elsewhere.
        if next_ch != '?' && is_expression_delimiter(next_ch) {
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

    Ok(())
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
            | '?'
            | ':'
            | '{'
            | '}'
            | '['
            | ']'
            | ','
            | ';'
            | '$'
    )
}

// LRM 3.7.1 simple identifier reader. The leading character (letter or
// underscore) has already been consumed; this gathers the rest of
// `[a-zA-Z0-9_$]*`.
fn read_simple_identifier<I>(first_ch: char, chars: &mut std::iter::Peekable<I>) -> Token
where
    I: Iterator<Item = (usize, char)>,
{
    let mut name = String::new();
    name.push(first_ch);
    while let Some((_, ch)) = chars.peek().copied() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
            chars.next();
            name.push(ch);
        } else {
            break;
        }
    }
    Token::Identifier(name)
}
