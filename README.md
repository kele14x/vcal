# VCAL

VCAL is a **V**erilog **cal**culator: an interactive REPL for evaluating Verilog expressions when writing or debugging Verilog code. It follows a subset of IEEE 1364-2005, focused on constants, expressions, and variables.

## Use cases

- Quickly test expression snippets and system functions
- Explore syntax and experiment with ideas
- Debug and inspect variables
- Use it as a calculator or learning tool

## Install & run

```sh
cargo run             # interactive REPL
cargo build           # build the binary
cargo test            # tests
```

## Example

```plain
In [0]: reg [7:0] a
In [1]: a = 4'hF + 4'hF
In [2]: a
Out[2]: 8'b00011110

In [3]: a + 4'b1
Out[3]: 8'b00011111

In [4]:
```

## Documentation

- [doc/repl.md](doc/repl.md) — prompt, session, exit behavior, lexical clarifications
- [doc/expressions.md](doc/expressions.md) — width, signedness, leaf extension, base propagation
- [doc/operators.md](doc/operators.md) — per-operator semantics
- [doc/variables.md](doc/variables.md) — `reg`, blocking assignment, bit-select / part-select
- [doc/non-standard.md](doc/non-standard.md) — vcal-specific divergences from LRM
- [doc/lrm-coverage.md](doc/lrm-coverage.md) — LRM chapter coverage and grammar matrix
- [doc/scope.md](doc/scope.md) — current status and backlog
- [doc/architecture.md](doc/architecture.md) — source layout and REPL entry points
