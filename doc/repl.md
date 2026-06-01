# REPL

## Prompt

- Prompt format is `In [n]: ` / `Out[n]: ` (trailing space), where `n` is the index of the n-th user input, starting from 0.
- `In [n]:` accepts a single line of Verilog. Multi-line input is a backlog item; see [scope.md](scope.md).
- Each input gets exactly one output slot, followed by a blank line so consecutive turns are visually separated. Following the IPython convention, the REPL prints:
  - `Out[n]: <value>` when the last statement is an expression and the input does not end with `;` — the value renders in canonical Verilog form `<width>'<base><digits>` (see [expressions.md](expressions.md) → "Base rules") — then a blank separator line before the next `In [n+1]:` prompt.
  - a bare blank line (acting as both the output and the separator) for everything else: declarations, assignments, system tasks (`$finish`, `$stop`), or any expression whose input ends with `;`. Trailing `;` is the IPython-style suppression marker; see [non-standard.md](non-standard.md).
  - an error message followed by a blank separator line, on evaluation failure. The `In [n]` counter still advances.

## Session

Declarations and assignments persist across REPL turns. Assignments don't echo a value (use a follow-up expression to read the variable back):

```plain
In [0]: reg [7:0] a
In [1]: a = 4'hF + 4'hF
In [2]: a
Out[2]: 8'b00011110

In [3]: a + 4'b1
Out[3]: 8'b00011111

In [4]:
```

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
