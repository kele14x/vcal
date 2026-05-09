# TODO

We are currently on Phase 1.

## Implementation Plan

Phase 1 target:

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

Things that do implement later:

- Numbers constants
  - Real numbers
- Variables
  - Variable declarations
- Expression
  - Operators
- String
- Vectors
- Other system tasks and functions

## Improvements
