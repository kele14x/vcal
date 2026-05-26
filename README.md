# VCAL

VCAL is a **V**erilog **cal**culator app that help Verilog developers to evaluate some expression when writing code or debugging.

Use cases:

- Quickly test expression snippets and system functions
- Explore syntax and experiment with ideas
- Debug and inspect variables
- Use it as a calculator or learning tool

The app is a commandline calculator. It works like a REPL loop for a limit subset of verilog syntax. Generally it follows Verilog LRM "IEEE Standard for Verilog Hardware Description Language" (IEEE Std 1364-2005). However it only focus the constants, expression and variables related part.

## Supported Matrix

This is final support target matrix, not means currently supported or implemented. The checked box means support, uncheck mean not support since not need.

- [x] 3. Lexical conventions
  - [x] 3.1 Lexical tokens
  - [x] 3.2 White spaces
  - [ ] 3.3 Comments
  - [x] 3.4 Operators
  - [x] 3.5 Numbers
  - [x] 3.6 Strings
  - [x] 3.7 Identifiers, keywords, and system names
    - [ ] 3.7.1 Escaped identifiers
    - [x] 3.7.2 Keywords (partly supported)
    - [x] 3.7.3 System tasks and functions (partly supported)
  - [ ] 3.8 Attributes
- [x] 4. Data types
  - [x] 4.1 Value set
  - [x] 4.2 Nets and variables
    - [x] 4.2.1 Net declarations
    - [x] 4.2.2 Variable declarations (reg only; integer/real/time deferred)
  - [x] 4.3 Vectors
  - [ ] 4.4 Strengths
  - [ ] 4.5 Implicit declarations
  - [ ] 4.6 Net types
  - [x] 4.7 Regs
  - [x] 4.8 Integers, reals, times and realtimes
  - [x] 4.9 Arrays
  - [ ] 4.10 Parameters
  - [ ] 4.11 Name spaces
- [x] 5. Expressions
  - [x] 5.1 Operators
  - [x] 5.2 Operands
  - [ ] 5.3 Minimum, typical, and maximum delay expression
  - [x] 5.4 Expression bit lengths
  - [x] 5.5 Signed expression
  - [x] 5.6 Assignment and truncations
- [ ] 6. Assignments
  - [ ] 6.1 Continuous assignments
  - [x] 6.2 Procedural assignments
- [ ] 7. Gate- and switch-level modeling
- [ ] 8. User-defined primitives (UDPs)
- [ ] 9. Behavioral modeling
- [ ] 10. Tasks and functions
- [ ] 11. Scheduling semantics
- [ ] 12. Hierarchical structures
- [ ] 13. Configuring the contents of a design
- [ ] 14. Specify blocks
- [ ] 15. Timing checks
- [ ] 16. Backannotation using the standard delay format (SDF)
- [ ] 17 System tasks and functions
  - [x] 17.1 Display system task
  - [ ] 17.2 File input-output system tasks
  - [ ] 17.3 Timescale system task
  - [x] 17.4 Simulation control system tasks
  - [ ] 17.5 Programmable logic array (PLA) modeling system tasks
  - [ ] 17.6 Stochastic analysis tasks
  - [ ] 17.7 Simulation time system functions
  - [ ] 17.8 Conversion functions
  - [x] 17.9 Probabilistic distribution functions
  - [ ] 17.10 Command line input
  - [ ] 17.11 Math functions
- [ ] 18. Value change dump (VCD) files
- [ ] 19. Compiler directives

## Requirements

- [ ] General
  - [ ] TUI
  - [ ] Developed using Rust language

- [ ] Processing Sequence
  - [ ] Program Startup
    - [ ] To start Vcal REPL session, the user open terminal then type `vcal`, and press Enter. This launches the interactive shell.
    - [ ] On program startup, it prints the prompt `In[n]:` and let the user type expressions.
      - [ ] Where `n` is index of the n-th user input, start from 0
  - [ ] User type Verilog expression then press **Enter**
    - [ ] Program parse and evaluate the expression, then print the output to terminal after prompt `Out[n]:`.
      - [ ] There `n` is the index of corresponding user input
    - [ ] Let the user type multi-line express (ways of mulit-line editor is not clear yet)
    - [ ] The multi-line expression should be evaluated as line termination is still whitespace based on language LRM
  - [ ] To exit the Vcal, user use one of serval commands:
    - [ ] Type `$finish` or `$stop` then press **Enter**.
    - [ ] Press **Ctrl + D**
    - [ ] Press **Ctrl + C**

- [ ] Supported lexical tokens
  - [ ] White spaces
  - [ ] Operator
  - [ ] Number
    - [ ] Integer constants
    - [x] Real constants
    - [ ] Conversion
  - [ ] String
  - [ ] Identifier
  - [ ] Keyword

- [ ] Supported data types
  - [ ] Value set: 0/1/x/z
  - [ ] Variables
    - [ ] Variable declarations
  - [ ] Vectors

- [ ] Supported system tasks & functions
  - [ ] Supported system tasks
    - [ ] Display system tasks
      - [ ] `$display`
      - [ ] `$displayb`
      - [ ] `$displayo`
      - [ ] `$displayh`
    - [ ] Simulation control system task
      - [ ] `$finish`
      - [ ] `$stop`
  - [ ] Supported system functions
    - [ ] Sign-cast functions
      - [x] `$signed`
      - [x] `$unsigned`
    - [ ] Display-base cast functions
      - [x] `$bin`
      - [x] `$oct`
      - [x] `$dec`
      - [x] `$hex`
    - [ ] Conversion functions
      - [x] `$rtoi`
      - [x] `$itor`
      - [x] `$realtobits`
      - [x] `$bitstoreal`
    - [ ] Probabilistic distribution functions
      - [ ] `$random`
      - [ ] `$dist_uniform`
      - [ ] `$dist_normal`
      - [ ] `$dist_exponential`
      - [ ] `$dist_poisson`
      - [ ] `$dist_chi_square`
      - [ ] `$dist_t`
      - [ ] `$dist_erlang`
    - [ ] Math functions
      - [x] `$clog2`
      - [x] `$ln`
      - [x] `$log10`
      - [x] `$exp`
      - [x] `$sqrt`
      - [x] `$pow`
      - [x] `$floor`
      - [x] `$ceil`
      - [x] `$sin`
      - [x] `$cos`
      - [x] `$tan`
      - [x] `$asin`
      - [x] `$acos`
      - [x] `$atan`
      - [x] `$atan2`
      - [x] `$hypot`
      - [x] `$sinh`
      - [x] `$cosh`
      - [x] `$tanh`
      - [x] `$asinh`
      - [x] `$acosh`
      - [x] `$atanh`

- [ ] Supported operators
  - [x] `{}` Concatenation
  - [x] `{{}}` Replication
  - [x] unary `+` Unary positive
  - [x] unary `-` Unary negative
  - [x] `+` Arithmetic add
  - [x] `-` Arithmetic minus
  - [x] `*` Arithmetic multiply
  - [x] `/` Arithmetic divide
  - [x] `**` Arithmetic power
  - [x] `%` Modulus
  - [x] `>` Relational larger than
  - [x] `>=` Relational larger or equal than
  - [x] `<` Relational less than
  - [x] `<=` Relational less or equal than
  - [x] `!` Logical negation
  - [x] `&&` Logical and
  - [x] `||` Logical or
  - [x] `==` Logical equality
  - [x] `!=` Logical inequality
  - [x] `===` Case equality
  - [x] `!==` Case inequality
  - [x] `~` Bitwise negation
  - [x] `&` Bitwise and
  - [x] `|` Bitwise inclusive or
  - [x] `^` Bitwise exclusive or
  - [x] `^~` or `~^` Bitwise equivalence
  - [x] `&` Reduction and
  - [x] `~&` Reduction nand
  - [x] `|` Reduction or
  - [x] `~|` Reduction nor
  - [x] `^` Reduction xor
  - [x] `~^` or `^~` Reduction xnor
  - [x] `<<` Logical left shift
  - [x] `>>` Logical right shift
  - [x] `<<<` Arithmetic left shift
  - [x] `>>>` Arithmetic right shift
  - [x] `? :` Conditional

- [ ] Supported syntax definition
  - [ ] A.2 Declarations
    - [ ] A.2.1 Declaration types
      - [ ] A.2.1.3 Type declarations
        - [ ] integer_declaration ::= integer list_of_variable_identifiers ;
        - [ ] real_declaration ::= real list_of_real_identifiers ;
        - [x] reg_declaration ::= reg [ signed ] [ range ] list_of_variable_identifiers ;
        - [ ] time_declaration ::= time list_of_variable_identifiers ;
    - [ ] A.2.2 Declaration data types
      - [ ] A.2.2.1 Net and variable types
        - [ ] real_type ::= real_identifier { dimension }
                          | real_identifier = constant_expression
        - [ ] variable_type ::= variable_identifier { dimension }
                              | variable_identifier = constant_expression
    - [ ] A.2.3 Declaration lists
      - [ ] list_of_real_identifiers ::= real_type { , real_type }
      - [x] list_of_variable_identifiers ::= variable_type { , variable_type }
    - [ ] A.2.5 Declaration ranges
      - [ ] dimension ::= [ dimension_constant_expression : dimension_constant_expression ]
      - [x] range ::= [ msb_constant_expression : lsb_constant_expression ]
  - [ ] A.6 Behavioral statements
    - [x] A.6.2 Procedural blocks and assignments
      - [x] blocking_assignment ::= variable_lvalue = expression
      - [ ] variable_assignment ::= variable_lvalue = expression
    - [x] A.6.4 Statements
      - [x] statement ::= blocking_assignment ;
  - [ ] A.8 Expression
    - [ ] A.8.1 Concatenations
      - [ ] concatenation ::= { expression { , expression } }
      - [ ] constant_concatenation ::= { constant_expression { , constant_expression } }
      - [ ] constant_multiple_concatenation ::= { constant_expression constant_concatenation }
      - [ ] multiple_concatenation ::= { constant_expression concatenation }
    - [ ] A.8.2 Function calls
      - [ ] constant_system_function_call ::= system_function_identifier ( constant_expression { , constant_expression } )
      - [ ] system_function_call ::= system_function_identifier [ ( expression { , expression } ) ]
    - [ ] A.8.3 Expressions
      - [ ] base_expression ::= expression
      - [ ] conditional_expression ::= expression1 ? expression2 : expression3
      - [ ] constant_base_expression ::= constant_expression
      - [ ] constant_expression ::= constant_primary
                                  | unary_operator constant_primary
                                  | constant_expression binary_operator constant_expression
                                  | constant_expression ? constant_expression : constant_expression
      - [ ] constant_range_expression ::= constant_expression
                                        | msb_constant_expression : lsb_constant_expression
                                        | constant_base_expression +: width_constant_expression
                                        | constant_base_expression -: width_constant_expression
      - [ ] dimension_constant_expression ::= constant_expression
      - [ ] expression ::= primary
                         | unary_operator primary
                         | expression binary_operator expression
                         | conditional_expression
      - [ ] expression1 ::= expression
      - [ ] expression2 ::= expression
      - [ ] expression3 ::= expression
      - [ ] lsb_constant_expression ::= constant_expression
      - [ ] msb_constant_expression ::= constant_expression
      - [ ] range_expression ::= expression
                               | msb_constant_expression : lsb_constant_expression
                               | base_expression +: width_constant_expression
                               | base_expression -: width_constant_expression
      - [ ] width_constant_expression ::= constant_expression
    - [ ] A.8.4 Primaries
      - [ ] constant_primary ::= number
                               | constant_concatenation
                               | constant_multiple_concatenation
                               | constant_system_function_call
                               | string
      - [ ] primary ::= number
                      | identifier [ { [ expression ] } [ range_expression ] ]
                      | concatenation
                      | multiple_concatenation
                      | system_function_call
                      | string
    - [ ] A.8.5 Expression left-side values
      - [ ] variable_lvalue ::= variable_identifier [ { [ expression ] } [ range_expression ] ]
                              | { variable_lvalue { , variable_lvalue } }
    - [ ] A.8.6 Operators
      - [ ] unary_operator ::= + | - | ! | ~ | & | ~& | | | ~| | ^ | ~^ | ^~
      - [ ] binary_operator ::= + | - | * | / | % | == | != | === | !== | && | || | **
                              | < | <= | > | >= | & | | | ^ | ^~ | ~^ | >> | << | >>> | <<<
    - [ ] A.8.7 Numbers
      - [ ] number ::= decimal_number
                     | octal_number
                     | binary_number
                     | hex_number
                     | real_number
      - [ ] real_number ::= unsigned_number . unsigned_number
                          | unsigned_number [ . unsigned_number ] exp [ sign ] unsigned_number
      - [ ] exp ::= e | E
      - [ ] decimal_number ::= unsigned_number
                             | [ size ] decimal_base unsigned_number
                             | [ size ] decimal_base x_digit { _ }
                             | [ size ] decimal_base z_digit { _ }
      - [ ] binary_number ::= [ size ] binary_base binary_value
      - [ ] octal_number ::= [ size ] octal_base octal_value
      - [ ] hex_number ::= [ size ] hex_base hex_value
      - [ ] sign ::= + | -
      - [ ] size ::= non_zero_unsigned_number
      - [ ] non_zero_unsigned_number ::= non_zero_decimal_digit { _ | decimal_digit}
      - [ ] unsigned_number ::= decimal_digit { _ | decimal_digit }
      - [ ] binary_value ::= binary_digit { _ | binary_digit }
      - [ ] octal_value ::= octal_digit { _ | octal_digit }
      - [ ] hex_value ::= hex_digit { _ | hex_digit }
      - [ ] decimal_base ::= '[s|S]d | '[s|S]D
      - [ ] binary_base ::= '[s|S]b | '[s|S]B
      - [ ] octal_base ::= '[s|S]o | '[s|S]O
      - [ ] hex_base ::= '[s|S]h | '[s|S]H
      - [ ] non_zero_decimal_digit ::= 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9
      - [ ] decimal_digit ::= 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9
      - [ ] binary_digit ::= x_digit | z_digit | 0 | 1
      - [ ] octal_digit ::= x_digit | z_digit | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7
      - [ ] hex_digit ::= x_digit | z_digit | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9
                        | a | b | c | d | e | f | A | B | C | D | E | F
      - [ ] x_digit ::= x | X
      - [ ] z_digit ::= z | Z | ?
    - [ ] A.8.8 Strings
      - [ ] string ::= " { Any_ASCII_Characters_except_new_line } "
  - A.9 General
    - A.9.3 Identifiers
      - [ ] escaped_identifier ::= \ { Any_ASCII_character_except_white_space } white_space
      - [x] identifier ::= simple_identifier
                         | escaped_identifier
      - [ ] real_identifier ::= identifier
      - [x] simple_identifier ::= [ a-zA-Z_ ] { [ a-zA-Z0-9_$ ] }
      - [x] system_function_identifier ::= $[ a-zA-Z0-9_$ ]{ [ a-zA-Z0-9_$ ] }
      - [x] system_task_identifier ::= $[ a-zA-Z0-9_$ ]{ [ a-zA-Z0-9_$ ] }
      - [x] variable_identifier ::= identifier
    - [ ] A.9.4 White space
      - [ ] white_space ::= space | tab | newline | eof

- [ ] Supported Keyword
  - [ ] integer
  - [ ] real
  - [x] reg
  - [ ] integer
  - [x] signed

## Main Gap

- Ways of multi-line editor is not clear yet
- Malformed real literals like `1._0` or `9.` surface as `invalid decimal digits: 1.0` after the underscore-strip / digit-strip step, because the lexer's `real_after_dot` lookahead requires `.` followed by a digit and otherwise falls through to the integer path. The diagnostic is correct in spirit (the literal is not a valid real) but the message is misleading. A future pass should recognize "digit-run + `.`" as a real-literal commitment and emit a real-specific error.

## Detailed Implementation

### Prompt

- Prompt format is `In[n]: ` / `Out[n]: ` (trailing space).
- The `Out[n]: ` print the expression return value or the number value.
  - Print `Out[n]: ` with an empty value for system tasks like `$finish`/`$stop`.
  - It should print result in a canonical Verilog form like `<width>'<base><digits>`. The expression should preserve source base then possible.

### Identifier white spaces

There rules follows LRM but not directly written so noted here:

- *real_number*, *non_zero_unsigned_number*, *unsigned_number*, *binary_value*, *octal_value*, *hex_value*, *decimal_base*, *binary_base*, *octal_base*, *hex_base* does not allow embedded spaces.
- A simple_identifier shall start with an alpha or underscore (`_`) character, shall have at least one character, and shall not have any spaces.
- The dollar sign (`$`) in a *system_function_identifier* or *system_task_identifier* shall not be followed by white space.

Based on LRM, there could be spaces between the 3 tokens (size, base, value) of integer constants. For example `8 'd 5` is the same as `8'd5`. However there should be no spaces between the `'` and the base (`b`, `o`, `d`, `h`, `sb`, `so`, `sd`, `sh`). Also there should be not spaces between `s` and the base.

### Base rules

The integer implementation should holds at least 4-fields for the features specified in LRM.

- Width
- Signed
- Bits (value)

However we need additional field for proper display in console:

- Base

The base of an arithmetic result is inferred from its operands so the output keeps the form the user typed when possible. The LRM does not specify this — it is a vcal display convention.

- A literal carries the base it was declared with. Unsized decimal literals (e.g. `42`) are decimal.
- A unary operator (`+`, `-`) preserves the operand's base. So `-4'b1` is `4'b1111`.
- A binary operator (`+`, `-`, `*`, `/`, `%`, `**`) takes the **leftmost** operand's base. So `4'b0111 + 4'b1001` is `4'b0000`, `8'h0a + 8'b1` is `8'h0b`, and `8'b00001010 + 8'h05` is `8'b00001111`.
  - The leftmost-wins rule mirrors the left-to-right evaluation order of the supported operators. There is no automatic base "promotion" between bases.

### Exit behavior

- The system task `$finish` and `$stop` both ends the REPL.
- The `EOF` char also ends the REPL.
- `Ctrl + C` ends the REPL.
- `Ctrl + D` ends the REPL.

### Session

The declarations and assignments persist across REPL session. For example:

```plain
In[0]: reg [7:0] a
Out[0]:
In[1]: a = 4'hF + 4'hF
Out[1]: 8'b00011110
In[2]: a + 4'b1
Out[2]: 8'b00011111
```

### Reg variables and blocking assignment

The only variable type vcal currently declares is `reg` (LRM A.2.1.3):

```text
reg [signed] [range] name { , name }
```

- The display base for a reg is binary, so a reg renders as
  `<width>'b<digits>` (signed: `<width>'sb<digits>`). `reg [7:0] a` reads
  back as `8'bxxxxxxxx` before any assignment.
- An unsized decl is 1 bit (`reg a` → `1'bx`).
- Range halves are constant integer expressions evaluated in the current
  session at decl time; they must be non-negative and free of x/z bits.
  A reversed range (`reg [0:7] a`) yields the same width as its normal
  form per LRM 4.8.
- Multiple names can share one decl: `reg [3:0] a, b, c`.
- Redeclaring an existing identifier in the same session replaces the
  prior binding — the REPL is single-scope and a redecl is the user's way
  of resetting a reg's metadata, so the new decl wipes the old width,
  signedness, base, and bit pattern. The freshly redeclared reg starts at
  all `x` like any other new reg.
- A fresh reg is initialized to all `x`. The decl statement emits an empty
  `Out[n]:` line — the same convention `$finish` / `$stop` use for
  non-value statements.

Blocking assignment `name = expression` is a top-level statement, not an
expression (LRM A.6.2), so it does not nest inside larger expressions.
The LHS reg's width, signedness, and base flow into the RHS via the
standard §5.6 context rules, then the resulting bits replace the reg's
bits while the reg's declared metadata is preserved. A real-typed RHS
goes through an implicit real→integer conversion per LRM §3.5.3 (round
to nearest, ties away from zero — the same rule `$itor`'s internal
real→int step uses); NaN / ±∞ have no integer image and surface as the
lvalue filled with x bits at its declared width. `Out[n]:` prints the
reg's new canonical form in its own display base.

An identifier reference resolves to the reg's current bits and then
participates in the surrounding expression like any other primary — its
`(width, signed, base)` propagates per §5.5 (so e.g. an 8-bit binary reg
on the left of `+` makes the result render in binary). Referencing an
undeclared name is an error.

### Bit-select and part-select

A declared reg can be sliced four ways (LRM 4.2.1 / 5.2.1 / 5.2.2):

| Syntax        | Form                       | Result width  |
|---------------|----------------------------|---------------|
| `r[expr]`     | bit-select                 | 1             |
| `r[m:l]`      | part-select                | `|m-l|+1`     |
| `r[b +: w]`   | indexed part-select up     | `w`           |
| `r[b -: w]`   | indexed part-select down   | `w`           |

The select grammar only attaches to a declared identifier — `4'b1111[0]`
does not parse, matching the LRM production
`identifier [ { [ expression ] } [ range_expression ] ]`. LHS selects
(`r[0] = 1'b1`) and the unpacked `{ dimension }` array form remain out
of scope.

Per LRM 5.2.1, "A bit-select or part-select of a scalar … shall be
illegal." A reg declared without a range is a scalar even when its
effective width is 1, so all four select forms on it are an error.
`reg [0:0] a` is a 1-bit *vector*, on the other hand, and accepts the
same selects any other vector does.

Every select is unsigned (LRM 4.7) regardless of the source reg's
signedness, and the result inherits the reg's display base. The width is
fixed by the form, so the select acts as a leaf primary: outer-context
width still widens it (zero-extension, since the result is unsigned),
but the index / base / endpoint sub-expressions are self-determined.

Unlike the LRM's elaboration-oriented constant-expression rules for
`[m:l]` and the `width` half of `+:` / `-:`, vcal evaluates all four
select forms at runtime against the current session state. So `m`, `l`,
`base`, and `width` may be ordinary integer expressions that reference
previously declared regs, as long as the final runtime values satisfy the
same semantic checks (`width > 0`, no x/z in places where the operation
needs a definite width, and constant-part-select direction matching the
declared reg direction). This fits vcal's REPL model: there is no
separate elaboration stage, so select operands are resolved when the line
is evaluated.

Source-index → internal-bit mapping is `internal = |src - lsb_decl|`,
which works uniformly across forward, reversed, and negative-endpoint
decls. For example, `reg [-1:2] r = 4'b1011` has width 4; `r[-1]` maps
to internal index `|-1 - 2| = 3` (the MSB end of the stored bits) and
`r[2]` maps to internal index `0` (the LSB end). For indexed
part-selects the source range is always numerically `[base, base+w-1]`
(for `+:`) or `[base-w+1, base]` (for `-:`) regardless of the reg's
declared direction; which end of that range becomes the result's MSB
depends on the declared direction (forward decl → larger source index
is more significant; reversed decl → smaller is more significant).

Two LRM clarifications worth pinning down:

1. **Strict direction on part-select.** LRM 5.2.1 says "the
   first expression shall address a more significant bit than the
   second", which uniquely fixes the legal direction relative to the
   reg's declared direction (`[m:l]` on `reg [7:0]` requires `m ≥ l`;
   on `reg [0:7]` requires `m ≤ l`). iverilog merely warns when the
   directions disagree; vcal errors, because the rule is unambiguous
   and silently reinterpreting the select hides a real bug.
2. **Out-of-range part-select bits are x per position.** LRM 4.2.1
   mandates that a bit-select with an out-of-range index returns `x`.
   For partial-overlap part-selects we apply the same rule one position
   at a time: each result bit whose source index falls outside the
   declared reg becomes `x`, and in-range bits keep their actual value.
   So `reg [3:0] a = 4'b0101; a[4:3]` is `2'bx0` — bit 4 is off the
   end, bit 3 is in range.

x/z bits in a runtime index or base flow through the result: a bit-select
with an unknown index is `1'bx`, and an indexed-part-select with an
unknown `base` fills the whole result with `x` (we don't know which
positions would have been in range). x/z bits in a part-select endpoint
or in an indexed-select `width` are an error instead, because those
positions must resolve to definite integers for the select shape to be
known; a `width` of zero or negative is likewise rejected.

## Non-standard Behavior

### Trailing semicolons

The Verilog LRM requires a trailing semicolons for each statement. This is annoying for a calculator app. We should accept a optional trailing semicolons. Users could use a trailing semicolons to explicitly end the input phase and force the app to evaluate the input (works together with multi-line edit).

### Integer Constants

Unsized number (simple decimal number or a number without size) shall be at least 32 bits. We should use number of bits longer than 32 if the value needs more bits instead of strictly truncated to 32-bits based on LRM.

### Arithmetic operators

The LRM specifies any unknown bits will cause the arithmetic operator returns all `x`. However in almost all implementation (`iverilog`, etc.), the `unary +` will return the bit the same, including `x` and `z`. For other arithmetic operators, if any operand's any bit value is `x` or `z`, then the entire result value shall be all `x`.

### Bitwise operators

LRM 1364-2005 has an internal inconsistency about operand extension: §5.1.10 says "the shorter operand is zero-filled in the most significant bit positions", but §5.5.2 says a narrower operand is sign-extended whenever the propagated type is signed (which, by §5.5.1, happens when *all* operands are signed). For `4'shF | 8'sh0` the two rules disagree — §5.1.10 would give `8'sh0F`, §5.5.2 gives `8'shFF`. vcal follows §5.5.2 (sign-extend when both signed, zero-extend otherwise), matching iverilog, VCS, Xcelium, and the IEEE 1800 (SystemVerilog) clarification that drops the §5.1.10 sentence entirely. This is the same extension rule already used by relational/equality/arithmetic in vcal, so all operators stay consistent.

### Bit-select and part-select operands

The Verilog LRM requires constant-expression operands for `r[m:l]` and
for the `width` half of `r[b +: w]` / `r[b -: w]`, because simulators and
synthesizers resolve those shapes during elaboration. vcal has no
separate elaboration stage: the REPL evaluates each input directly
against the current `Session`. So vcal deliberately relaxes those forms
to ordinary integer expressions evaluated at runtime. The resulting
values still must be usable as a select shape: part-select endpoints and
indexed widths must resolve to definite integers, and indexed widths must
be positive.

### Real numbers

vcal stores real values as Rust `f64`, which is IEEE 754 binary64 — the same format LRM §3.5.2 references. A few corners the LRM leaves to the implementation are pinned down here:

- §5.1.5 says `0.0 ** ≤0` and `negative ** non-integral` are *unspecified* for real `**`. vcal returns whatever Rust's `f64::powf` produces:
  - `0.0 ** 0.0` → `1.0`
  - `0.0 ** -1.0` → `inf`
  - `(-2.0) ** 0.5` → `NaN`
  These come from IEEE 754 directly. iverilog and VCS may differ on the exact value, so don't rely on a specific corner result.
- `1'bx ? real_a : real_b` cannot reproduce the integer per-bit-merge rule (real has no per-bit identity). vcal returns the common branch value when both branches agree bit-for-bit on `f64::to_bits`, and `NaN` otherwise.
- Real values render in fixed-point for magnitudes in `[1e-4, 1e10)` and scientific notation outside that window — purely a display choice, not specified by the LRM.
- §17.8 doesn't address NaN / ±∞ in `$rtoi`. vcal returns 32 bits of `x` to surface "no defined integer image" rather than silently mapping to zero. Out-of-range finite values wrap mod 2³² (the same overflow rule the rest of the integer pipeline uses).
- §17.8 doesn't address NaN / ±∞ in `$itor` either. `$itor` on a real argument goes through implicit real→integer→real; the implicit real→int step has no integer image for NaN/±∞, so it yields `x` (matching the `$rtoi` rule above), and §3.5.3's int→real then maps every `x` bit to `0`. So `$itor(0.0/0.0)` and `$itor(±1.0/0.0)` all collapse to `0.0`, keeping `$itor` self-consistent with `$rtoi`.
- §17.8 doesn't carve out an x/z rule for `$bitstoreal`. vcal applies §3.5.3's "x/z → 0" rule to its 64-bit operand for consistency with the sibling integer-to-real conversions, so `$bitstoreal(64'bx)` decodes as `+0.0`.
- §17.11 doesn't address x/z bits in `$clog2`. vcal returns 32 bits of `x` whenever the operand contains any x or z bit, mirroring the `$rtoi` NaN/±∞ rule (surface "no defined image" rather than silently mapping to zero). Real arguments take the §3.5.3 round-half-away-from-zero path, so NaN/±∞ collapse to `32'sdx` the same way they do under `$rtoi`. Finite reals wrap mod 2³² before the unsigned interpretation, matching `$rtoi`'s 32-bit signed result domain. Per LRM the operand is "treated as an unsigned value" of its natural width, so `$clog2(64'hFFFF_FFFF_FFFF_FFFF)` is `32'sd64` and `$clog2(-1)` (32-bit signed) is `32'sd32`.

### Conditional operator

vcal deliberately diverges from LRM Table 5-21 on the ambiguous-cond merge. The strict table reduces *every* combination other than `(0,0)` and `(1,1)` to `x` — including `(x,x)` and `(z,z)`. iverilog (and most other simulators) instead use the value-preserving rule above, on the principle that if both branches put the same `x` (or `z`) at the same position regardless of cond, the result is necessarily that bit and reducing it to `x` would discard information. So `1'bx ? 4'b01xz : 4'b01xz` is `4'b01xz` here (and in iverilog), not the `4'b01xx` the LRM table prescribes. vcal follows iverilog as the practical reference.

### Display-base cast functions

vcal adds four non-standard system functions — `$bin`, `$oct`, `$dec`, `$hex` — that change only the display base of an integer expression. The argument is evaluated as a self-determined expression; the result has the same width, signedness, and bits, with `Base` overridden to the cast's target. Outer-context width still flows back through the cast per §5.5.2 (same shape as `$signed` / `$unsigned`). Real arguments are rejected — reals have no display base.

These exist so users do not need tricks like `1'b0 + 1` to render `1` in binary; `$bin(1)` does the job directly.

Because the argument is evaluated self-determined, the cast acts as a context barrier — outer-context width does *not* flow into the argument. So `$hex(4'hf + 4'hf) + 8'h0` is `8'h0e` (the inner `+` overflows at 4 bits, then extends), while the un-cast `(4'hf + 4'hf) + 8'h0` is `8'h1e` (the outer 8-bit context widens the inner `+` before computing). This matches the §5.5 self-determined-argument rule already used by `$signed` / `$unsigned` and is not specific to the display-base casts.
