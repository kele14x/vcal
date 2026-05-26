# LRM coverage

This is the final support target matrix and grammar coverage for IEEE 1364-2005, not a snapshot of what is currently implemented. Checked = supported (target). Unchecked = not supported (intentionally out of scope).

For "what works *today*" vs "what is the long-term target", see [scope.md](scope.md).

## Chapter checklist

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

## Supported operators

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

## Supported system tasks & functions

- [ ] Display system tasks
  - [ ] `$display`
  - [ ] `$displayb`
  - [ ] `$displayo`
  - [ ] `$displayh`
- [ ] Simulation control system task
  - [ ] `$finish`
  - [ ] `$stop`
- [ ] Sign-cast functions
  - [x] `$signed`
  - [x] `$unsigned`
- [ ] Display-base cast functions (vcal-specific; see [non-standard.md](non-standard.md))
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

## Supported syntax definitions

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
  - [x] A.8.5 Expression left-side values
    - [x] variable_lvalue ::= variable_identifier [ { [ expression ] } [ range_expression ] ]
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

## Supported keywords

- [ ] integer
- [ ] real
- [x] reg
- [x] signed
