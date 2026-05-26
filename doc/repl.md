# REPL

## Prompt

- Prompt format is `In[n]: ` / `Out[n]: ` (trailing space), where `n` is the index of the n-th user input, starting from 0.
- `In[n]:` accepts a single line of Verilog. Multi-line input is a backlog item; see [scope.md](scope.md).
- `Out[n]:` prints either:
  - the result in canonical Verilog form `<width>'<base><digits>` — the expression preserves its source base when possible (see [expressions.md](expressions.md) → "Base rules"), or
  - an empty value for non-value statements like declarations, `$finish`, and `$stop`.

## Session

Declarations and assignments persist across REPL turns:

```plain
In[0]: reg [7:0] a
Out[0]:
In[1]: a = 4'hF + 4'hF
Out[1]: 8'b00011110
In[2]: a + 4'b1
Out[2]: 8'b00011111
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
