# REPL

## Prompt

- Prompt format is `In [n]:` / `Out[n]:` (plus a trailing space), where `n` is the index of the n-th user input, starting from 0.
- `In [n]:` accepts a single line of Verilog. Multi-line input is a backlog item; see [scope.md](scope.md).
- Each input gets exactly one output slot, followed by a blank line so consecutive turns are visually separated. Following the IPython convention, the REPL prints:
  - `Out[n]: <values>` when the last statement is a non-empty `display_expression` (see [non-standard.md](non-standard.md) → "Top-level input") and the input does not end with `;`. Every top-level expression is a `display_expression`: `a` has one argument, while `a, b` has two. Without a leading format string, every value renders in canonical Verilog form `<width>'<base><digits>` (see [expressions.md](expressions.md) → "Base rules") and values are space-separated. In a multi-argument list, a string-style first argument activates the `$display` format-control engine, so `"a=%d", a` prints `a=10`; arguments left unconsumed by format controls retain canonical rendering, so `"label", 8'hff` prints `label 8'hff`. String style is preserved by string literals and string-only concatenation / replication, but is not inferred from the bits stored in a packed `reg`. The output is followed by a blank separator line before the next `In [n+1]:` prompt.
  - a bare blank line (acting as both the output and the separator) for everything else: declarations, assignments, system tasks (`$finish`, `$stop`), or any `display_expression` whose input ends with `;`. Trailing `;` is the IPython-style suppression marker; see [non-standard.md](non-standard.md).
  - an error message followed by a blank separator line, on evaluation failure. The `In [n]` counter still advances.

## Output encoding

`$display` / `$write` output and formatted `display_expression` echoes are a raw byte stream: `%s` / `%c` may emit arbitrary bytes, and piping or redirecting the REPL preserves them exactly. The one exception is a Windows console, whose stdio rejects non-UTF-8 writes; there, non-UTF-8 bytes are degraded to lossy UTF-8 (replacement characters) so the REPL keeps working. Valid UTF-8 output — which is everything except deliberate `%s` / `%c` raw bytes — is never altered.

## Session

Declarations and assignments persist across REPL turns. Assignments don't echo a value (use a follow-up expression to read the variable back):

```plain
In [0]: reg [7:0] a
In [1]: a = 4'hF + 4'hF
In [2]: a
Out[2]: 8'h1e

In [3]: a + 4'b1
Out[3]: 8'h1f

In [4]: a, $hex(a + 8'd1)
Out[4]: 8'h1e 8'h1f

In [5]: "a is %d", a
Out[5]: a is 30

In [6]:
```

The comma is a `display_expression` argument separator, not a Verilog comma
operator or tuple constructor. As in a `$display` argument list, each slot is an
optional expression, so null slots from leading, trailing, or adjacent commas
are accepted (each emits one space). Semicolons still sequence statements, so
assignment followed by inspection is written `a = 10; a, b`; `a = 10, a` is
rejected with a hint to use `;`.

A `display_expression` must contain at least one expression or comma token. A
blank line is empty input and produces no implicit display call; this is distinct
from the valid explicit zero-argument task call `$display()`.

A `Session` owns the variable map (`RegValue`, not just bare `IntegerValue`) and is threaded through every evaluator entry. Reg metadata (width, signedness, base, declared msb/lsb) survives across turns; only redeclaration replaces it. See [variables.md](variables.md) for details.

## Exit behavior

The REPL ends on any of:

- `$finish` or `$stop` (LRM 17.4 simulation control system tasks)
- `EOF` / `Ctrl-D`
- `Ctrl-C`

## Lexical clarifications

These rules follow LRM but are not directly written there, so they are noted here:

- *real_number*, *non_zero_unsigned_number*, *unsigned_number*, *binary_value*, *octal_value*, *hex_value*, *decimal_base*, *binary_base*, *octal_base*, *hex_base* do not allow embedded spaces.
- A *simple_identifier* shall start with an alpha or underscore (`_`), shall have at least one character, and shall not have any spaces.
- The dollar sign (`$`) in a *system_function_identifier* or *system_task_identifier* shall not be followed by white space.
- Based on LRM, spaces are allowed between the three tokens (size, base, value) of an integer constant: `8 'd 5` is the same as `8'd5`. However there shall be no spaces between the `'` and the base (`b`, `o`, `d`, `h`, `sb`, `so`, `sd`, `sh`), nor between `s` and the base.
