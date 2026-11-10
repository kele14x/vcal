use std::borrow::Cow;
use std::io::{self, IsTerminal};

use rustyline::Helper;
use rustyline::completion::Completer;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::validate::Validator;

use crate::highlight::{Span, TokenClass, highlight_spans};

// True when stdout is a real terminal and the user hasn't opted out via
// the NO_COLOR convention (https://no-color.org). Independent of the
// `stdin().is_terminal()` gate in `main.rs` because stdin and stdout can
// be redirected separately — color follows the stream that displays it.
pub(crate) fn should_color() -> bool {
    io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

pub(crate) fn green(s: &str) -> String {
    format!("\x1b[32m{s}\x1b[0m")
}

pub(crate) fn red(s: &str) -> String {
    format!("\x1b[31m{s}\x1b[0m")
}

fn token_color(class: TokenClass) -> Option<&'static [u8]> {
    match class {
        TokenClass::Number => Some(b"\x1b[33m"),
        // Same numeric hue, but faint so the base reads apart from the digits.
        TokenClass::BaseMarker => Some(b"\x1b[2;33m"),
        TokenClass::SystemIdent => Some(b"\x1b[35m"),
        TokenClass::String => Some(b"\x1b[36m"),
        TokenClass::Comment => Some(b"\x1b[2m"),
        TokenClass::Error => Some(b"\x1b[31m"),
        TokenClass::Identifier | TokenClass::Operator | TokenClass::Punct => None,
    }
}

// Shared palette renderer. Spans refer to the original byte buffer, not to
// previously inserted ANSI. Copy all other bytes verbatim: formatted text may
// contain arbitrary non-UTF-8 bytes before or between canonical value spans.
pub(crate) fn highlight_bytes<'a>(bytes: &'a [u8], spans: &[Span], enabled: bool) -> Cow<'a, [u8]> {
    if !enabled || spans.is_empty() {
        return Cow::Borrowed(bytes);
    }
    let mut out = Vec::with_capacity(bytes.len() + spans.len() * 12);
    let mut cursor = 0;
    for span in spans {
        out.extend_from_slice(&bytes[cursor..span.start]);
        if let Some(color) = token_color(span.class) {
            out.extend_from_slice(color);
            out.extend_from_slice(&bytes[span.start..span.end]);
            out.extend_from_slice(b"\x1b[0m");
        } else {
            out.extend_from_slice(&bytes[span.start..span.end]);
        }
        cursor = span.end;
    }
    out.extend_from_slice(&bytes[cursor..]);
    Cow::Owned(out)
}

// Minimal rustyline `Helper` whose only job is to wrap the main prompt
// in green via `highlight_prompt`. Going through the Highlighter
// pipeline (instead of embedding ANSI in the string passed to
// `readline`) lets rustyline track the prompt's display width itself,
// so cursor positioning and line redraws stay correct.
pub(crate) struct PromptHelper {
    pub enabled: bool,
}

impl Completer for PromptHelper {
    type Candidate = String;
}

impl Hinter for PromptHelper {
    type Hint = String;
}

impl Validator for PromptHelper {}

impl Highlighter for PromptHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if self.enabled && default {
            Cow::Owned(green(prompt))
        } else {
            Cow::Borrowed(prompt)
        }
    }

    // Token-based line highlight. Rustyline calls this on every full repaint;
    // `highlight_char` decides which edits force one. Uses the lenient
    // `highlight_spans` tokenizer so partial input doesn't flash red while
    // the user is typing — only truly unrecognized chars get the Error colour.
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if !self.enabled {
            return Cow::Borrowed(line);
        }
        match highlight_bytes(line.as_bytes(), &highlight_spans(line), true) {
            Cow::Borrowed(_) => Cow::Borrowed(line),
            Cow::Owned(bytes) => {
                Cow::Owned(String::from_utf8(bytes).expect("ANSI insertion preserves input UTF-8"))
            }
        }
    }

    // Edits must force a full repaint: inserting a char can recolour text
    // before it (`4` -> `4'`), and rustyline's no-repaint fast path would
    // print the new char uncoloured. Cursor moves need none, since colours
    // never depend on the cursor position; rustyline still does one repaint
    // on the first move after a `true` and plain cursor moves after that.
    fn highlight_char(&self, _line: &str, _pos: usize, kind: CmdKind) -> bool {
        self.enabled && kind != CmdKind::MoveCursor
    }
}

impl Helper for PromptHelper {}
