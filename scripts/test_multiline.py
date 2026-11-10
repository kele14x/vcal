#!/usr/bin/env python3
"""Exercise interactive editing in a Unix PTY after `cargo build`.

Usage: python3 scripts/test_multiline.py [--binary path/to/vcal]
Only Python's standard library is required. The PTY supplies a controlling
terminal and answers cursor-position queries; assertions inspect submitted
results rather than depending on rustyline's redraw sequences.
"""

import argparse
import errno
import fcntl
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import termios
import time


ENTER = b"\r"
ALT_ENTER = b"\x1b\r"
CTRL_J = b"\n"
UP = b"\x1b[A"
DOWN = b"\x1b[B"
LEFT = b"\x1b[D"
RIGHT = b"\x1b[C"
DELETE = b"\x1b[3~"
ANSI = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]")


class Terminal:
    def __init__(self, binary, *, parse_only=False, color=False, columns=80):
        self.binary = binary
        self.color = color
        self.raw = bytearray()
        self.cursor_queries = 0
        self.status = None
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.environ["TERM"] = "xterm-256color"
            if color:
                os.environ.pop("NO_COLOR", None)
            else:
                os.environ["NO_COLOR"] = "1"
            args = [str(binary)] + (["--parse-only"] if parse_only else [])
            os.execv(str(binary), args)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, columns, 0, 0))

    @property
    def output(self):
        return ANSI.sub(b"", self.raw).replace(b"\r", b"")

    def send(self, data):
        while data:
            data = data[os.write(self.fd, data):]

    def paste(self, text):
        self.send(b"\x1b[200~" + text + b"\x1b[201~")

    def read(self, timeout):
        if not select.select([self.fd], [], [], timeout)[0]:
            return
        try:
            chunk = os.read(self.fd, 65536)
        except OSError as err:
            if err.errno != errno.EIO:
                raise
            chunk = b""
        self.raw.extend(chunk)
        queries = self.raw.count(b"\x1b[6n")
        while self.cursor_queries < queries:
            self.send(b"\x1b[1;1R")
            self.cursor_queries += 1

    def wait_for(self, text, timeout=5):
        deadline = time.monotonic() + timeout
        while text not in self.output:
            if time.monotonic() >= deadline:
                raise AssertionError(f"did not receive {text!r}")
            self.read(min(0.1, deadline - time.monotonic()))

    def pending(self, index=0):
        deadline = time.monotonic() + 0.2
        while time.monotonic() < deadline:
            self.read(min(0.05, deadline - time.monotonic()))
        assert f"In [{index + 1}]: ".encode() not in self.output, "submitted before Enter"
        assert f"Out[{index}]: ".encode() not in self.output, "evaluated before Enter"
        assert b"Syntax error:" not in self.output, "parsed before Enter"

    def submit_value(self, value, index=0):
        self.send(ENTER)
        self.wait_for(f"In [{index + 1}]: ".encode())
        expected = f"Out[{index}]: {value}\n".encode()
        assert expected in self.output, f"missing result {expected!r}"
        assert self.output.count(f"Out[{index}]: ".encode()) == 1, "multiple output slots"

    def close(self):
        # Ctrl-D ends both modes when they are waiting at an empty prompt.
        try:
            self.send(b"\x04")
            deadline = time.monotonic() + 1
            while time.monotonic() < deadline:
                pid, status = os.waitpid(self.pid, os.WNOHANG)
                if pid:
                    self.status = status
                    return
                self.read(0.05)
            os.kill(self.pid, signal.SIGKILL)
            _, self.status = os.waitpid(self.pid, 0)
        finally:
            os.close(self.fd)


def typed_and_history(terminal):
    terminal.send(b"1+" + ALT_ENTER + b"2")
    terminal.pending()
    terminal.submit_value("32'sd3")
    terminal.send(UP)
    terminal.pending(1)
    terminal.submit_value("32'sd3", 1)


def newline_at_cursor(terminal):
    terminal.send(b"1+2" + LEFT + ALT_ENTER)
    terminal.pending()
    terminal.submit_value("32'sd3")


def ctrl_j_newline(terminal):
    terminal.send(b"1+" + CTRL_J + b"2")
    terminal.pending()
    terminal.submit_value("32'sd3")


def ctrl_j_at_cursor(terminal):
    terminal.send(b"1+2" + LEFT + CTRL_J)
    terminal.pending()
    terminal.submit_value("32'sd3")


def mixed_newline_shortcuts(terminal):
    terminal.send(b"1+" + CTRL_J + b"2+" + ALT_ENTER + b"3")
    terminal.pending()
    terminal.submit_value("32'sd6")
    terminal.send(UP)
    terminal.submit_value("32'sd6", 1)


def backslash_enter_is_not_continuation(terminal):
    terminal.send(b"1+\\" + ENTER)
    terminal.wait_for(b"In [1]: ")
    assert b"Syntax error:" in terminal.output
    assert b"Out[0]: " not in terminal.output
    terminal.send(b"42")
    terminal.submit_value("32'sd42", 1)


def edit_earlier_line(terminal):
    terminal.paste(b"1\n+2")
    terminal.send(UP + LEFT + DELETE + b"9")
    terminal.pending()
    # Enter submits even though the cursor is on the first line.
    terminal.submit_value("32'sd11")


def up_and_down(terminal):
    terminal.paste(b"1+\n2")
    terminal.send(UP + DOWN + b"\x7f3")
    terminal.submit_value("32'sd4")


def left_and_right_across_newline(terminal):
    terminal.paste(b"1+\n2")
    terminal.send(LEFT + LEFT + RIGHT + DELETE + b"3")
    terminal.submit_value("32'sd4")


def paste_statements_and_comments(terminal):
    terminal.paste(b'integer a=5;\n// ] in a comment\na=(a+\n2);\n"a=%d",a')
    terminal.pending()
    terminal.submit_value("a=7")
    terminal.send(b"a")
    terminal.submit_value("32'sd7", 1)


def paste_crlf_and_suppression(terminal):
    terminal.paste(b"1+\r\n2;\r\n")
    terminal.pending()
    terminal.send(ENTER)
    terminal.wait_for(b"In [1]: ")
    assert b"Out[0]: " not in terminal.output, "trailing semicolon did not suppress echo"
    assert b"error:" not in terminal.output
    terminal.send(b"42")
    terminal.submit_value("32'sd42", 1)


def incomplete_submission(terminal):
    terminal.send(b"1+" + ENTER)
    terminal.wait_for(b"In [1]: ")
    assert b"Syntax error: unexpected end of expression" in terminal.output
    terminal.send(b"42")
    terminal.submit_value("32'sd42", 1)


def newline_is_not_statement_separator(terminal):
    terminal.paste(b"1\n2")
    terminal.send(ENTER)
    terminal.wait_for(b"In [1]: ")
    assert b"Syntax error: unexpected token after end of statement" in terminal.output
    assert b"Out[0]: " not in terminal.output


def colored_wrapped_buffer(terminal):
    terminal.paste(b"integer abcdefghijk=1;\n/* multi\nline */\nabcdefghijk+\n2")
    terminal.pending()
    terminal.submit_value("32'sd3")
    assert b"\x1b[33m" in terminal.raw, "number highlighting missing"


RESET = b"\x1b[0m"
YELLOW = b"\x1b[33m"
DIM_YELLOW = b"\x1b[2;33m"
CYAN = b"\x1b[36m"


def colored_number(size, marker, digits):
    return YELLOW + size + RESET + DIM_YELLOW + marker + RESET + YELLOW + digits + RESET


ESCAPED_STRING = rb'"hello\n\"world\\end"'
CANONICAL_RESULTS = [
    (b"8'hff, 4'b0011, 8'd42",
     colored_number(b"8", b"'h", b"ff") + b" "
     + colored_number(b"4", b"'b", b"0011") + b" "
     + colored_number(b"8", b"'d", b"42")),
    (b"8'hxz, 4'b10xz, 8'dx, 8'dz",
     colored_number(b"8", b"'h", b"xz") + b" "
     + colored_number(b"4", b"'b", b"10xz") + b" "
     + colored_number(b"8", b"'d", b"x") + b" "
     + colored_number(b"8", b"'d", b"z")),
    (b"-1", b"-" + colored_number(b"32", b"'sd", b"1")),
    # A real may include its sign in the yellow span; an integer must not.
    (b"-1.25", (YELLOW + b"-1.25" + RESET, b"-" + YELLOW + b"1.25" + RESET)),
    (ESCAPED_STRING, CYAN + ESCAPED_STRING + RESET),
    (b', 1,, 4\'hf, "hi",',
     b" " + colored_number(b"32", b"'sd", b"1") + b" "
     + colored_number(b"4", b"'h", b"f") + b" " + CYAN + b'"hi"' + RESET + b" "),
]
FORMATTED_RESULTS = [
    (b'"8\'hff %s %h", "4\'b10xz", 8\'hff', b"8'hff 4'b10xz ff"),
    (b'"label", 8\'hff', b"label " + colored_number(b"8", b"'h", b"ff")),
    (b'"value=%d", 8\'hff, 4\'b1010, "8\'hff"',
     b"value=255 " + colored_number(b"4", b"'b", b"1010")
     + b" " + CYAN + b'"8\'hff"' + RESET),
    (b'"%s", 8\'ha9, 8\'hff', b"\xa9 " + colored_number(b"8", b"'h", b"ff")),
]


def output_prefix(terminal, index):
    prefix = f"Out[{index}]: ".encode()
    return b"\x1b[31m" + prefix + RESET if terminal.color else prefix


def raw_result(terminal, index):
    # Anchor to the complete Out prefix, not input redraws with the same colors.
    prefix = output_prefix(terminal, index)
    raw = bytes(terminal.raw)
    assert raw.count(prefix) == 1, f"missing or duplicate output prefix {prefix!r}"
    result, separator, _ = raw.split(prefix, 1)[1].partition(b"\r\n\r\n")
    assert separator, f"unterminated output slot {index}"
    return result.replace(b"\r\n", b"\n")


def submit_raw_result(terminal, source, expected, index=0):
    terminal.paste(source)
    terminal.send(ENTER)
    terminal.wait_for(f"In [{index + 1}]: ".encode())
    result = raw_result(terminal, index)
    alternatives = expected if isinstance(expected, tuple) else (expected,)
    if not terminal.color:
        alternatives = tuple(ANSI.sub(b"", value) for value in alternatives)
        assert b"\x1b" not in result, f"NO_COLOR styled output slot {index}: {result!r}"
    assert result in alternatives, f"{source!r}: expected {alternatives!r}, got {result!r}"
    assert terminal.output.count(f"Out[{index}]: ".encode()) == 1, "multiple output slots"


def canonical_result_colors(terminal):
    for index, (source, expected) in enumerate(CANONICAL_RESULTS):
        submit_raw_result(terminal, source, expected, index)


def formatted_result_colors(terminal):
    for index, (source, expected) in enumerate(FORMATTED_RESULTS):
        submit_raw_result(terminal, source, expected, index)


def no_color_results(terminal):
    # Only inspect output slots: rustyline's cursor/control ANSI is allowed elsewhere.
    for index, (source, expected) in enumerate(CANONICAL_RESULTS + FORMATTED_RESULTS):
        submit_raw_result(terminal, source, expected, index)


def explicit_task_output_unchanged(terminal):
    for index, (task, ending) in enumerate(((b"$display", b"\r\n"), (b"$write", b""))):
        source = task + b'("task 8\'hff %s", "4\'b10xz", 8\'hff); 8\'hff'
        submit_raw_result(terminal, source, colored_number(b"8", b"'h", b"ff"), index)
        # Anchor after the accepted-input newline and before the red Out prefix.
        # Rustyline may disable bracketed paste here; permit control CSI, not SGR (m).
        expected = b"task 8'hff 4'b10xz 255" + ending + output_prefix(terminal, index)
        boundary = rb"\r\n(?:\x1b\[[0-?]*[ -/]*[@-ln-~])*"
        assert re.search(boundary + re.escape(expected), terminal.raw), f"{task!r} output was styled or changed"


def redirected_run(binary, source, *, parse_only=False):
    env = os.environ.copy()
    env["TERM"] = "xterm-256color"
    env.pop("NO_COLOR", None)
    result = subprocess.run(
        [str(binary)] + (["--parse-only"] if parse_only else []),
        input=source, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, timeout=5,
    )
    assert result.returncode == 0, f"redirected process failed: {result.stderr!r}"
    return result.stdout


def colored_parse_only_unchanged(terminal):
    source = b'8\'hff, "8\'hff", 1+2'
    plain = redirected_run(terminal.binary, source + b"\n", parse_only=True)
    prefix, suffix = b"In [0]: Out[0]: ", b"\n\nIn [1]: "
    assert plain.startswith(prefix) and plain.endswith(suffix), f"unexpected AST output {plain!r}"
    expected = plain[len(prefix):-len(suffix)]
    assert b"ReplEcho" in expected and b"StringLiteral" in expected and b"Binary {" in expected
    terminal.paste(source)
    terminal.send(ENTER)
    terminal.wait_for(b"In [1]: ")
    result = raw_result(terminal, 0)
    # Ignore PTY carriage returns just as Terminal.output does; the AST itself
    # has only LF newlines, and output spanning terminal-mode changes may have
    # extra CRs. Keep raw byte checks strict for the formatter cases above.
    assert result.replace(b"\r", b"") == expected, "parse-only AST content changed"
    assert b"\x1b" not in result, "parse-only AST was highlighted"


def stdout_redirection(binary):
    source = (b"8'hff, 4'b0011, 8'd42\n"
              b'"%s", 8\'ha9, 8\'hff\n'
              b'$display("task 8\'hff %s", "4\'b10xz"); '
              b'$write("write 8\'hff"); 8\'hff\n$finish\n')
    expected = (b"In [0]: Out[0]: 8'hff 4'b0011 8'd42\n\n"
                b"In [1]: Out[1]: \xa9 8'hff\n\n"
                b"In [2]: task 8'hff 4'b10xz\nwrite 8'hffOut[2]: 8'hff\n\nIn [3]: \n")
    output = redirected_run(binary, source)
    assert output == expected, f"redirected stdout changed: {output!r}"
    assert b"\x1b" not in output, "redirected stdout was styled without NO_COLOR"


def parse_only_typed(terminal):
    terminal.send(b"1+" + ALT_ENTER + b"2")
    terminal.pending()
    terminal.send(ENTER)
    terminal.wait_for(b"In [1]: ")
    assert b"Out[0]: " in terminal.output and b"Binary {" in terminal.output
    assert b"Syntax error:" not in terminal.output


def parse_only_paste(terminal):
    terminal.paste(b"integer a=1;\na+\n2")
    terminal.pending()
    terminal.send(ENTER)
    terminal.wait_for(b"In [1]: ")
    assert b"Decl {" in terminal.output and b"Binary {" in terminal.output
    assert terminal.output.count(b"Out[0]: ") == 1
    assert b"Syntax error:" not in terminal.output


def parse_only_ctrl_j(terminal):
    terminal.send(b"1+" + CTRL_J + b"2")
    terminal.pending()
    terminal.send(ENTER)
    terminal.wait_for(b"In [1]: ")
    assert b"Out[0]: " in terminal.output and b"Binary {" in terminal.output
    assert b"Syntax error:" not in terminal.output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[1] / "target/debug/vcal")
    binary = parser.parse_args().binary.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error(f"build vcal first; executable not found at {binary}")

    cases = [
        ("Alt-Enter and whole-buffer history", typed_and_history, {}),
        ("Alt-Enter at the cursor", newline_at_cursor, {}),
        ("Ctrl-J inserts a newline and Ctrl-M still submits", ctrl_j_newline, {}),
        ("Ctrl-J at the cursor", ctrl_j_at_cursor, {}),
        ("mixed newline shortcuts and history", mixed_newline_shortcuts, {}),
        ("backslash-Enter is not continuation", backslash_enter_is_not_continuation, {}),
        ("edit an earlier line and submit there", edit_earlier_line, {}),
        ("Up / Down within the buffer", up_and_down, {}),
        ("Left / Right across a newline", left_and_right_across_newline, {}),
        ("paste statements, comments and a formatted echo", paste_statements_and_comments, {}),
        ("CRLF paste and trailing semicolon", paste_crlf_and_suppression, {}),
        ("Enter submits incomplete syntax", incomplete_submission, {}),
        ("newlines do not separate statements", newline_is_not_statement_separator, {}),
        ("colored multi-line buffer with terminal wrapping", colored_wrapped_buffer, {"color": True, "columns": 20}),
        ("canonical output colors, unknowns, signs, strings and null spacing", canonical_result_colors, {"color": True}),
        ("plain formatted output and colored canonical leftovers", formatted_result_colors, {"color": True}),
        ("explicit display/write output remains plain", explicit_task_output_unchanged, {"color": True}),
        ("NO_COLOR leaves all output regions plain", no_color_results, {}),
        ("colored parse-only preserves the plain AST", colored_parse_only_unchanged, {"parse_only": True, "color": True}),
        ("parse-only Alt-Enter", parse_only_typed, {"parse_only": True}),
        ("parse-only multi-line paste", parse_only_paste, {"parse_only": True}),
        ("parse-only Ctrl-J", parse_only_ctrl_j, {"parse_only": True}),
    ]
    for name, check, options in cases:
        terminal = Terminal(binary, **options)
        try:
            terminal.wait_for(b"In [0]: ")
            check(terminal)
        except Exception:
            print(f"FAIL {name}")
            print(repr(terminal.output[-4000:]))
            raise
        finally:
            terminal.close()
        assert terminal.status == 0, f"{name}: process did not exit cleanly ({terminal.status})"
        print(f"PASS {name}")
    stdout_redirection(binary)
    print("PASS redirected stdout remains plain without NO_COLOR")
    print(f"All {len(cases)} interactive checks and stdout redirection passed.")


if __name__ == "__main__":
    main()
