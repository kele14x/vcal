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

- Ways of multi-line editor is not clear yet

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
In[0]: integer a = 3
Out[0]: 3
In[1]: a + 2
Out[1]: 5
```

## Non-standard Behavior

### Trailing semicolons

The Verilog LRM requires a trailing semicolons for each statement. This is annoying for a calculator app. We should accept a optional trailing semicolons. Users could use a trailing semicolons to explicitly end the input phase and force the app to evaluate the input (works together with multi-line edit).

### Integer Constants

Unsized number (simple decimal number or a number without size) shall be at least 32 bits. We should use number of bits longer than 32 if the value needs more bits instead of strictly truncated to 32-bits based on LRM.

### Arithmetic operators

The LRM specifies any unknown bits will cause the arithmetic operator returns all `x`. However in almost all implementation (`iverilog`, etc.), the `unary +` will return the bit the same, including `x` and `z`. For other arithmetic operators, if any operand's any bit value is `x` or `z`, then the entire result value shall be all `x`.

### Bitwise operators

LRM 1364-2005 has an internal inconsistency about operand extension: §5.1.10 says "the shorter operand is zero-filled in the most significant bit positions", but §5.5.2 says a narrower operand is sign-extended whenever the propagated type is signed (which, by §5.5.1, happens when *all* operands are signed). For `4'shF | 8'sh0` the two rules disagree — §5.1.10 would give `8'sh0F`, §5.5.2 gives `8'shFF`. vcal follows §5.5.2 (sign-extend when both signed, zero-extend otherwise), matching iverilog, VCS, Xcelium, and the IEEE 1800 (SystemVerilog) clarification that drops the §5.1.10 sentence entirely. This is the same extension rule already used by relational/equality/arithmetic in vcal, so all operators stay consistent.

### Conditional operator

vcal deliberately diverges from LRM Table 5-21 on the ambiguous-cond merge. The strict table reduces *every* combination other than `(0,0)` and `(1,1)` to `x` — including `(x,x)` and `(z,z)`. iverilog (and most other simulators) instead use the value-preserving rule above, on the principle that if both branches put the same `x` (or `z`) at the same position regardless of cond, the result is necessarily that bit and reducing it to `x` would discard information. So `1'bx ? 4'b01xz : 4'b01xz` is `4'b01xz` here (and in iverilog), not the `4'b01xx` the LRM table prescribes. vcal follows iverilog as the practical reference.
