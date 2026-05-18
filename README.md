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
    - [ ] 4.2.2 Variable declarations
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
    - [ ] Real constants
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
    - [ ] Conversion functions
      - [ ] `$rtoi`
      - [ ] `$itor`
      - [ ] `$realtobits`
      - [ ] `$bitstoreal`
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
      - [ ] `$clog2`
      - [ ] `$ln`
      - [ ] `$log10`
      - [ ] `$exp`
      - [ ] `$sqrt`
      - [ ] `$pow`
      - [ ] `$floor`
      - [ ] `$ceil`
      - [ ] `$sin`
      - [ ] `$cos`
      - [ ] `$tan`
      - [ ] `$asin`
      - [ ] `$acos`
      - [ ] `$atan`
      - [ ] `$atan2`
      - [ ] `$hypot`
      - [ ] `$sinh`
      - [ ] `$cosh`
      - [ ] `$tanh`
      - [ ] `$asinh`
      - [ ] `$acosh`
      - [ ] `$atanh`

- [ ] Supported operators
  - [ ] `{}` Concatenation
  - [ ] `{{}}` Replication
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
        - [ ] reg_declaration ::= reg [ signed ] [ range ] list_of_variable_identifiers ;
        - [ ] time_declaration ::= time list_of_variable_identifiers ;
    - [ ] A.2.2 Declaration data types
      - [ ] A.2.2.1 Net and variable types
        - [ ] real_type ::= real_identifier { dimension }
                          | real_identifier = constant_expression
        - [ ] variable_type ::= variable_identifier { dimension }
                              | variable_identifier = constant_expression
    - [ ] A.2.3 Declaration lists
      - [ ] list_of_real_identifiers ::= real_type { , real_type }
      - [ ] list_of_variable_identifiers ::= variable_type { , variable_type }
    - [ ] A.2.5 Declaration ranges
      - [ ] dimension ::= [ dimension_constant_expression : dimension_constant_expression ]
      - [ ] range ::= [ msb_constant_expression : lsb_constant_expression ]
  - [ ] A.6 Behavioral statements
    - [ ] A.6.2 Procedural blocks and assignments
      - [ ] blocking_assignment ::= variable_lvalue = expression
      - [ ] variable_assignment ::= variable_lvalue = expression
    - [ ] A.6.4 Statements
      - [ ] statement ::= blocking_assignment ;
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
      - [ ] identifier ::= simple_identifier
                         | escaped_identifier
      - [ ] real_identifier ::= identifier
      - [ ] simple_identifier ::= [ a-zA-Z_ ] { [ a-zA-Z0-9_$ ] }
      - [ ] system_function_identifier ::= $[ a-zA-Z0-9_$ ]{ [ a-zA-Z0-9_$ ] }
      - [ ] system_task_identifier ::= $[ a-zA-Z0-9_$ ]{ [ a-zA-Z0-9_$ ] }
      - [ ] variable_identifier ::= identifier
    - [ ] A.9.4 White space
      - [ ] white_space ::= space | tab | newline | eof

- [ ] Supported Keyword
  - [ ] integer
  - [ ] real
  - [ ] reg
  - [ ] integer
  - [ ] signed

## Main Gap

- [ ] Ways of multi-line editor is not clear yet
- [ ] The full expression width/sign rules across all Verilog operators are not fully implemented yet

## Detailed Implementation

### 4-Value logic

Verilog LRM 3.5 Numbers specified numbers as *integer constants* or *real constants*. Integer constants follows the 4-value logic. That is integer constants are always has size and could be split into multiply bits. Each bits could be 0/1/x/z. Real constants follows IEEE Std 754-1985, so no 4-value logic.

### Trailing semicolons

The Verilog LRM requires a trailing semicolons for each statement. This is annoying for a calculator app. We should accept a optional trailing semicolons. Users could use a trailing semicolons to explicitly end the input phase and force the app to evaluate the input (works together with multi-line edit).

### Identifier white spaces

*real_number*, *non_zero_unsigned_number*, *unsigned_number*, *binary_value*, *octal_value*, *hex_value*, *decimal_base*, *binary_base*, *octal_base*, *hex_base* does not allow embedded spaces.

A simple_identifier shall start with an alpha or underscore (`_`) character, shall have at least one character, and shall not have any spaces.

The dollar sign (`$`) in a *system_function_identifier* or *system_task_identifier* shall not be followed by white space.

### Constants

#### Integer Constants

Integer constants are mainly specific in LRM section "3.5.1 Integer constants". It mainly be divided into two types:

- Simple decimal number
- Based constant, which be composed by up to three tokens: a optional size constant, an apostrophe character (`'`) followed by a base format character, and the digits representing the value of the number.

Imports notes (which follows LRM but specified here as notes):

- Unsized number (simple decimal number or a number without size) shall be at least 32 bits, but may be longer than 32 if the value needs more bits
- If the value digits occupy fewer bits than the literal width (or fewer than 32 bits for an unsized literal), the value is left-extended. Ordinary unsigned digits are zero-extended, `x` digits are extended with `x`, and `z`/`?` digits are extended with `z`. This literal-digit padding rule is not sign extension.
- There could be spaces between the 3 tokens (size, base, value) of integer constants. For example `8 'd 5` is the same as `8'd5`. However there should be no spaces between the `'` and the base (`b`, `o`, `d`, `h`, `sb`, `so`, `sd`, `sh`). Also there should be not spaces between `s` and the base.

### Operator precedence

Operator precedence follows IEEE 1364-2005 exactly.

Notes:

- All operators shall associate left to right with the exception of the conditional operator, which shall associate right to left.
  - The `**` operator is still left to right association, for example `3 ** 3 ** 3 = (3 ** 3) ** 3 = 19683`. Which is different from Python (`3 ** 3 ** 3 = 7625597484987`).
- Unary `+`/`-` bind tighter than `**`, for example `-2 ** 2 == 4`.
- The binary bitwise band sits between equality and `&&` per LRM Table 5-4, with internal order `&` > `^` `~^` `^~` > `|` (tightest to loosest).
- There is short-circuiting during expression evaluation.

### Width rules

Width rules follows IEEE 1364-2005 exactly.

Notes:

- Mainly on LRM section 5.4.
- Generally there are two types o expression bit lengths rules:
  - Self-determined expression: where the bit length of the expression is solely determined by the expression itself.
    - For example the `<` operator returns 1-bit unsigned result in all case. So the `a < b` expression is bit-width self-determined.
  - Context-determined expression: where the bit length of the expression is determined itself and the fact that it is part of another expression.
    - For example, the `=` statement, the bit size of the right-hand expression of an assignment depends on itself and the size of the left-hand side (LHS).
- vcal evaluates expressions in two passes to satisfy these rules:
  - First pass: bottom-up self-determined width/sign inference for each AST node.
  - Second pass: top-down context-determined evaluation so a parent expression can widen child arithmetic before truncation. Required for cases like `(a + b) + 0`, where the outer expression width widens the inner arithmetic before overflow is applied.
- For arithmetic operators (`+`, `-`, `*`, `/`, `%`, `**`), if any operand contains `x` or `z`, the result becomes all-`x` bits at the effective result width.
- The exponent operand of `**` is treated as self-determined.

### Signedness rules

Signedness rules follows IEEE 1364-2005 exactly.

Notes:

- Mainly on LRM section 5.5.
- Unlike the bit-width, signedness depends only on the operands.
- (Simple) decimal numbers are signed.
- Some operators are self-determined
  - For example, `<` result is always unsigned (also always 1-bit)
- Some operators are not self-determined

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
- All-`x` results inherit the same base. For wide non-decimal results this can be verbose (e.g. `4'bx + 1` prints as `32'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx`); this is intentional, matching how literals like `4'bx` already render as `4'bxxxx`.

### Operators

#### Arithmetic operators

There are 8 arithmetic operators in Verilog: 2 unary operators: `unary +` and `unary -`; 6 binary operators: `+`, `-`, `*`, `/`, `%` and `**`.

They handle `x` and `z` value in a very clear way: the `unary +` will return the bit the same, including `x` and `z`. For other operators, if any operand's any bit value is `x` or `z`, then the entire result value shall be all `x`.

#### Relational operators

There are 4 relational operators: `<`, `>`, `<=` and `>=`. The result is always a 1-bit unsigned number: `0`/`1`/`x`. If either operand contains any `x` or `z` bit, the result is `1'bx`.

Operand unification follows LRM 1364-2005 §5.5.2 and is shared with the equality operators. The relational expression first decides a single propagated context for both operands:

- **Width** = `max(L(lhs), L(rhs))`.
- **Signedness** = signed iff *both* operands are signed; otherwise unsigned.

That propagated context is pushed down through any context-determined sub-expressions (e.g. unary `-`, binary `+`/`*`/...) until it reaches a leaf primary. Extension at the primary is decided by the **propagated** signedness, not by the primary's own signedness:

- Propagated context is **unsigned** → narrower side is **zero-extended**, regardless of whether that side was originally signed.
- Propagated context is **signed** → narrower side is **sign-extended**.

Once both operands are at the same width under the unified type, the comparison is performed as integers in that type. The 1-bit result is independent of the rest of the surrounding expression (LRM §5.5.2 last paragraph).

Practical consequences (verified against iverilog as the golden reference):

- `4'sb1111 < 8'd255` → `1'b1`. The signed `4'sb1111` zero-extends to `8'b00001111` (= 15) under the unsigned context — *not* `8'b11111111`.
- `-4'sb1000 < 8'd9` → `1'b0`. Propagation passes through unary `-` to the primary `4'sb1000`, which zero-extends to `0000_1000` = 8; negation at 8-bit unsigned wraps to `1111_1000` = 248; 248 < 9 is false.

#### Equality operators

There are 4 equality operators: logical `==` and `!=`, and case `===` and `!==`. The result is always a 1-bit unsigned number. Equality is one precedence level *lower* than relational (LRM §5.1.8).

Operand unification is identical to the relational operators (see above). After unification:

- **`==` / `!=`** (logical equality) follow LRM §5.1.8. The result is `1'bx` only when the relation is *ambiguous*. Concretely:
  - If any bit position has a *definite* mismatch (one side is `0`, the other side is `1`), the operands are definitely unequal regardless of any `x`/`z` elsewhere — `==` returns `0`, `!=` returns `1`.
  - Otherwise, if any bit position involves `x` or `z`, the result is `1'bx`.
  - Otherwise, all bits match and the operands are equal.
- **`===` / `!==`** (case equality) compare the two operands bit-for-bit, treating `x` as matching only `x` and `z` as matching only `z`. The result is always a known `0` or `1`, never `x`.

Width-extension on `===` / `!==` follows LRM §5.5.4: the special "fill with `x`/`z` when the sign bit is `x`/`z`" rule applies *only* when the propagated context is signed (i.e. both operands are signed). Under an unsigned propagated context the narrower side is always zero-extended, even if its MSB is `x` or `z`. Examples (iverilog-confirmed):

- `4'sbx000 === 8'sbxxxxx000` → `1'b1` (both signed → x-fill).
- `4'sbx000 === 8'b0000x000` → `1'b1` (mixed → unsigned context → zero-fill).
- `4'sbx000 === 8'bxxxxx000` → `1'b0` (mixed; the upper `xxxx` of RHS does not match the zero-filled upper bits of LHS).

#### Logical operators

There are 3 logical operators: `&&`, `||` and `!`. `!` is unary operator so has higher precedence. The precedence of `&&` is greater than `||`. The result of logical operators is always 1-bit unsigned so they are very like relational and equality operators.

#### Bitwise operators

There are 4 binary bitwise operators — `&`, `|`, `^`, and `~^` (with `^~` accepted as an equivalent spelling for the equivalence operator) — and 1 per-bit unary operator `~`. They operate position-by-position on the operand bits.

- Width = `max(L(lhs), L(rhs))` for binaries; width = operand width for unary `~`. They are context-determined like arithmetic, so a wider parent context widens the per-bit operation before truncation.
- Signedness = signed iff *both* operands are signed (binaries); unary `~` preserves operand signedness.
- Base inheritance follows the existing rules: leftmost-wins for binaries, operand-preserving for unary `~`.
- 4-state truth tables per LRM §5.1.10 (verified bit-for-bit against iverilog in `doc/four_value_ops_output.txt`):
  - `~`: `0→1`, `1→0`, `x→x`, `z→x`.
  - `&`: a definite `0` on either side forces `0`; otherwise any `x`/`z` forces `x`; else AND.
  - `|`: a definite `1` on either side forces `1`; otherwise any `x`/`z` forces `x`; else OR.
  - `^`, `~^`/`^~`: any `x`/`z` bit forces `x`; otherwise XOR / XNOR.

LRM 1364-2005 has an internal inconsistency about operand extension: §5.1.10 says "the shorter operand is zero-filled in the most significant bit positions", but §5.5.2 says a narrower operand is sign-extended whenever the propagated type is signed (which, by §5.5.1, happens when *all* operands are signed). For `4'shF | 8'sh0` the two rules disagree — §5.1.10 would give `8'sh0F`, §5.5.2 gives `8'shFF`. vcal follows §5.5.2 (sign-extend when both signed, zero-extend otherwise), matching iverilog, VCS, Xcelium, and the IEEE 1800 (SystemVerilog) clarification that drops the §5.1.10 sentence entirely. This is the same extension rule already used by relational/equality/arithmetic in vcal, so all operators stay consistent.

Note that `~^` and `^~` always denote the same operator, while `~&` and `~|` exist only as unary reduction forms (no binary form).

#### Reduction operators

There are 6 unary reduction operators — `&`, `~&`, `|`, `~|`, `^`, and `~^` (with `^~` as the equivalent spelling for `~^`). They fold the binary 4-state truth tables of `&`/`|`/`^` across all bits of a single operand to produce a 1-bit result.

- The operand is self-determined (LRM Table 5-22); the 1-bit unsigned result widens through outer arithmetic context the same way as `!`, `&&`, `||`, relational, and equality results.
- Result base is always Binary, independent of operand base.
- `&`/`|`/`^`/`~^`/`^~` are also binary bitwise operators — they're disambiguated by parse position (unary when no left-hand operand is in scope).
- `~&` and `~|` exist only as unary reduction operators; using them in a binary position is a syntax error.
- Fold semantics per LRM §5.1.11, derived from the binary truth tables (verified against `doc/four_value_ops_output.txt`):
  - `&` (AND-reduction): a definite `0` anywhere forces `0`; otherwise any `x`/`z` forces `x`; else `1`.
  - `|` (OR-reduction): a definite `1` anywhere forces `1`; otherwise any `x`/`z` forces `x`; else `0`.
  - `^` (XOR-reduction): any `x`/`z` bit forces `x`; otherwise odd-parity (`1` for an odd count of `1` bits, else `0`).
  - `~&`/`~|`/`~^`/`^~`: the bitwise NOT of the corresponding positive reduction.

#### Shift operators

There are 4 shift operators — `<<` (logical left), `<<<` (arithmetic left), `>>` (logical right), and `>>>` (arithmetic right). Per LRM §5.1.12 the LHS is context-determined like arithmetic, while the RHS is self-determined and "always treated as an unsigned number ... has no effect on the signedness of the result". Concretely:

- Width = `max(L(lhs), L(context))`. The LHS widens to that result width before the shift runs, so an outer arithmetic context can rescue bits that a self-determined shift would have shifted out.
- Signedness = LHS signedness combined with the propagated outer context (LRM §5.5.1: any unsigned operand in the surrounding expression makes the result unsigned). The RHS never contributes.
- The RHS is read as an unsigned bit pattern regardless of its declared signedness, so a "negative" shift count (e.g. `-4'sd1`) becomes a very large unsigned value. The shift count is clamped to the result width — anything ≥ width yields the all-fill output, which keeps `BigUint`-sized counts safe and matches the saturated semantics implementations already give.
- If the RHS contains any `x` or `z` bit, the entire result is all-`x` (LRM §5.1.12 last paragraph).
- Vacated bits are filled with `0` for `<<`, `<<<`, and `>>`. For `>>>` the fill is the LHS's most-significant bit (after extension) when the propagated result type is signed, and `0` otherwise. So `4'sb1000 >>> 1` is `4'sb1100` self-determined, but `(4'sb1000 >>> 1) + 8'd0` is `8'b00000100` because the unsigned `8'd0` flips the propagated context to unsigned.
- Result base inherits from the LHS (leftmost-wins), like the other binary operators.

LHS bits that are not shifted out keep their value, including `x` and `z`, so `4'b01x0 << 1` is `4'b1x00`.

#### Conditional operator

Verilog's only ternary operator, written `expression1 ? expression2 : expression3` (LRM §5.1.13). It sits between `||` and the lowest precedence level (LRM Table 5-4) and is right-associative, so `a ? b : c ? d : e` parses as `a ? b : (c ? d : e)`.

- `expression1` is **self-determined** and reduced to a 1-bit logical the same way `&&`/`||`/`!` reduce their operands: any `1` is true, all `0` is false, otherwise (any `x`/`z` and no definite `1`) the cond is ambiguous and reduces to `x`.
- `expression2` and `expression3` are **context-determined** — both take the result width and signedness, with their leaf primaries extended accordingly.
- Width = `max(L(expression2), L(expression3), L(context))`.
- Signedness combines per §5.5.1 (any unsigned operand → unsigned result). As with shifts, the propagated outer signedness overrides the operand-derived signedness, so a mixed unsigned outer context zero-fills both branches at their leaves rather than sign-filling.
- When the cond is a definite `0` or `1`, only the chosen branch's bits feed the result. When the cond is `x`/`z`, both branches are evaluated and merged per bit: agreeing bits stay (so `x`∩`x` is `x` and `z`∩`z` is `z`), disagreeing bits become `x` (so `1`∩`0` and `x`∩`0` both yield `x`).
- Result base inherits from `expression2` (the leftmost bit-pattern operand after the cond), matching the leftmost-wins rule used elsewhere.

vcal deliberately diverges from LRM Table 5-21 on the ambiguous-cond merge. The strict table reduces *every* combination other than `(0,0)` and `(1,1)` to `x` — including `(x,x)` and `(z,z)`. iverilog (and most other simulators) instead use the value-preserving rule above, on the principle that if both branches put the same `x` (or `z`) at the same position regardless of cond, the result is necessarily that bit and reducing it to `x` would discard information. So `1'bx ? 4'b01xz : 4'b01xz` is `4'b01xz` here (and in iverilog), not the `4'b01xx` the LRM table prescribes. vcal follows iverilog as the practical reference, the same call already made for `>>>` fill behavior in the shift section.

Width extension of the two branches has the same §5.1.13-vs-§5.5.2 inconsistency the bitwise section already calls out. §5.1.13's last paragraph says the shorter operand "shall be lengthened to match the longer and zero-filled from the left", but §5.5.2 says a narrower operand is sign-extended whenever the propagated type is signed (which, by §5.5.1, happens when both branches are signed). For `1 ? 4'shF : 8'sh0` the two rules disagree — §5.1.13 would give `8'sh0f`, §5.5.2 gives `8'shff`. vcal follows §5.5.2 and matches iverilog, exactly as we do for bitwise: sign-extend when both branches are signed, zero-extend otherwise. So `1 ? 4'shF : 8'sh0` is `8'shff` and `1 ? 4'shF : 8'h0` is `8'h0f` (the unsigned `8'h0` flips the propagated context to unsigned, restoring zero-fill).

### Packed vs unpacked array

Packed vs unpacked array are in support target.

### Partial selects

partial selects and indexed part selects are in support target.

### System task and output format

The display system tasks `$display`, `$displayb`, `$displayo`, `$displayh` should print the number in the format specified in LRM.

The `Out[n]:` print the expression return value or the number value, or nothing for system tasks (no return value). It should print result in a canonical Verilog form like `<width>'<base><digits>`. The expression should preserve source base then possible.

### Exit behavior

- The system task `$finish` and `$stop` both ends the REPL.
- The `EOF` char also ends the REPL.
- `Ctrl + C` ends the REPL.
- `Ctrl + D` ends the REPL.

### Session

The declarations and assignments persist across REPL session. For example:

```plain
In[0]: integer a = 3
Out[0]: 3
In[1]: a + 2
Out[1]: 5
```
