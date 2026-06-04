use std::borrow::Cow;
use std::io::{self, IsTerminal};

use rustyline::Helper;
use rustyline::completion::Completer;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::validate::Validator;

use crate::highlight::{TokenClass, highlight_spans};

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

pub(crate) fn yellow(s: &str) -> String {
    format!("\x1b[33m{s}\x1b[0m")
}

pub(crate) fn magenta(s: &str) -> String {
    format!("\x1b[35m{s}\x1b[0m")
}

pub(crate) fn dim(s: &str) -> String {
    format!("\x1b[2m{s}\x1b[0m")
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

    // Token-based line highlight. Re-runs on every keystroke (gated by
    // `highlight_char` returning true). Uses the lenient `highlight_spans`
    // tokenizer so partial input doesn't flash red while the user is
    // typing — only truly unrecognized chars get the Error colour.
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if !self.enabled {
            return Cow::Borrowed(line);
        }
        let spans = highlight_spans(line);
        // Minimal palette: identifiers / operators / punct stay default so
        // the prompt remains quiet on busy terminals; only the high-signal
        // classes get a colour.
        let mut out = String::with_capacity(line.len() + spans.len() * 8);
        let mut cursor = 0usize;
        for span in spans {
            // Whitespace between spans is preserved verbatim.
            if span.start > cursor {
                out.push_str(&line[cursor..span.start]);
            }
            let s = &line[span.start..span.end];
            match span.class {
                TokenClass::Number => out.push_str(&yellow(s)),
                TokenClass::SystemIdent => out.push_str(&magenta(s)),
                TokenClass::Comment => out.push_str(&dim(s)),
                TokenClass::Error => out.push_str(&red(s)),
                TokenClass::Identifier | TokenClass::Operator | TokenClass::Punct => {
                    out.push_str(s)
                }
            }
            cursor = span.end;
        }
        if cursor < line.len() {
            out.push_str(&line[cursor..]);
        }
        Cow::Owned(out)
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _kind: CmdKind) -> bool {
        self.enabled
    }
}

impl Helper for PromptHelper {}
