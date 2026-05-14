# TODO

We are currently on Phase 2.

## Implementation Plan

### Phase 1 target

- [x] REPL shell with `In[n]:` and `Out[n]:`
- [x] Support whitespace and integer number
  - [x] Support all integer number format in LRM:
    - [x] Unsized decimal
    - [x] Unsized based integer (for example `'hFF)
    - [x] Signed/based format
    - [x] Underscore in digits
    - [x] `x/z/?` in digits
- [x] 4-value model for integers
- [x] Output format (`Out[n]:`) for phase 1:
  - [x] Print a canonical Verilog form like `<width>'<base><digits>`
  - [x] If no return value to print, just print `Out[n]:` with newline
- [x] Handles `$finish` and `$stop`
  - [x] Support both `$finish()` and `$finish` syntax since all are legal in LRM
- [x] Single line input only for phase 1
- [x] Optional tailing `;`
- [x] No operators, no variables, no strings, no real numbers yet

### Phase 2 target

Scope for Phase 2:

- Single-line REPL input only
- Integer literals and parentheses
- Integer arithmetic operators from IEEE 1364-2005 section 5.1.5:
  - unary `+`
  - unary `-`
  - `+`
  - `-`
  - `*`
  - `/`
  - `%`
  - `**`
- No variables, declarations, strings, real numbers, concatenation, or non-arithmetic operators

Implementation checklist:

- [x] Parse expressions with a real tokenizer and AST
- [x] Support parentheses and IEEE operator precedence for the Phase 2 arithmetic subset
- [x] Evaluate literals and grouped expressions through the AST path
- [x] Implement unary `+`
- [x] Implement unary `-`
- [x] Implement binary `+`
- [x] Implement binary `-`
- [x] Implement binary `*`
- [x] Implement binary `/`
- [x] Implement binary `%`
- [x] Implement binary `**`
- [x] Implement bottom-up self-determined width/sign inference for the arithmetic subset
- [x] Implement top-down context-determined evaluation so parent width can widen child arithmetic
- [x] Propagate `x`/`z` through arithmetic operators as all-`x` result bits at the effective result width
- [x] Implement signed/unsigned extension rules for the arithmetic subset
- [x] Add unit tests for precedence, width truncation, context widening, signedness, unknown propagation, and zero-division/undefined power cases

Implementation notes:

- Arithmetic expressions are implemented with a parsed AST, not ad hoc string splitting.
- Width handling uses a two-pass design:
  - First pass: infer the self-determined width and signedness of each AST node.
  - Second pass: evaluate with top-down context so a parent expression can widen child arithmetic before truncation.
- This is required for expressions such as `(a + b) + 0`, where the outer expression width widens the inner arithmetic before overflow is applied.
- For arithmetic operators, if any operand contains `x` or `z`, the result becomes all `x` bits at the effective result width.
- The exponent operand of `**` is treated as self-determined.

Things to implement later:

- Number constants
  - Real numbers
- Variables
  - Variable declarations
- Expressions
  - Non-arithmetic operators
- Strings
- Vectors
- Other system tasks and functions

## Improvements

- Add more LRM edge-case tests as new operators and expression forms are implemented
- Update this file first when the active phase changes or a phase task is completed
