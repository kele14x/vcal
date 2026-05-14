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
  - [ ] `>` Relational larger than
  - [ ] `>=` Relational larger or equal than
  - [ ] `<` Relational less than
  - [ ] `<=` Relational less or equal than
  - [ ] `!` Logical negation
  - [ ] `&&` Logical and
  - [ ] `||` Logical or
  - [ ] `==` Logical equality
  - [ ] `!=` Logical inequality
  - [ ] `===` Case equality
  - [ ] `!==` Case inequality
  - [ ] `~` Bitwise negation
  - [ ] `&` Bitwise and
  - [ ] `|` Bitwise inclusive or
  - [ ] `^` Bitwise exclusive or
  - [ ] `^~` or `~^` Bitwise equivalence
  - [ ] `&` Reduction and
  - [ ] `~&` Reduction nand
  - [ ] `|` Reduction or
  - [ ] `~|` Reduction nor
  - [ ] `^` Reduction xor
  - [ ] `~^` or `^~` Reduction xnor
  - [ ] `<<` Logical left shift
  - [ ] `>>` Logical right shift
  - [ ] `<<<` Arithmetic left shift
  - [ ] `>>>` Arithmetic right shift
  - [ ] `? :` Conditional

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

The (default) base rule should be designed.

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
