use std::io;

use rustyline::history::DefaultHistory;
use rustyline::{Cmd, Config, Editor, KeyCode, KeyEvent, Modifiers};

use crate::color::PromptHelper;

// Normal and parse-only interactive modes edit one buffer per input. Newlines
// are explicit: submitting an unfinished expression still reports a syntax
// error through the usual parser path.
pub(crate) fn interactive_editor(
    use_color: bool,
) -> io::Result<Editor<PromptHelper, DefaultHistory>> {
    let config = Config::builder().bracketed_paste(true).build();
    let mut editor = Editor::with_config(config).map_err(io::Error::other)?;
    editor.set_helper(Some(PromptHelper { enabled: use_color }));
    editor.bind_sequence(KeyEvent(KeyCode::Enter, Modifiers::NONE), Cmd::AcceptLine);
    editor.bind_sequence(KeyEvent(KeyCode::Enter, Modifiers::ALT), Cmd::Newline);
    editor.bind_sequence(KeyEvent::ctrl('J'), Cmd::Newline);
    // Only the Windows backend decodes a distinct shifted Enter event.
    #[cfg(windows)]
    editor.bind_sequence(KeyEvent(KeyCode::Enter, Modifiers::SHIFT), Cmd::Newline);
    // Rustyline's default arrows move within the buffer before navigating
    // history at its first/last line. Bracketed paste inserts all pasted lines
    // without submitting them.
    Ok(editor)
}
