use std::borrow::Cow;
use std::io::{self, IsTerminal};

use rustyline::Helper;
use rustyline::completion::Completer;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;

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
}

impl Helper for PromptHelper {}
