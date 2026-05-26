# AGENTS.md

## Commands

- `cargo test` — run tests
- `cargo run` — interactive REPL
- `cargo build` — build the binary

## Documentation boundary

- [README.md](README.md) — stable user-facing entry point. Do not edit without user input.
- [doc/scope.md](doc/scope.md) — mutable working state (current status, active scope, backlog). **Update this first** when scope changes or a task completes; collapse completed work to one-line summaries. Git history is the granular record.
- [doc/*.md](doc/) — stable implementation / spec docs (one topic per file: `repl`, `expressions`, `operators`, `variables`, `non-standard`, `lrm-coverage`, `architecture`).

Quick test for where a fact belongs: if it will still hold after the next feature ships, put it in one of the stable docs; otherwise it belongs in `doc/scope.md`.

## Meta-rules

- Add LRM edge-case tests as new operators land.
- Most design rules derive from LRM. Where vcal diverges intentionally, the divergence is documented in [doc/non-standard.md](doc/non-standard.md) — consult it before reading the LRM.
- Do not infer scope from [doc/lrm-coverage.md](doc/lrm-coverage.md) — many checked boxes are long-term targets, not current scope. Confirm with the user before expanding beyond what [doc/scope.md](doc/scope.md) lists as active.
- Two REPL entry points: `vcal::run_interactive` (rustyline, TTY only) and `vcal::run_repl(BufRead, Write)` (piped / test). `src/main.rs` dispatches via `IsTerminal`. Keep both paths working.
