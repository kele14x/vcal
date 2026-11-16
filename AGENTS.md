# AGENTS.md

## Commands

- `cargo test` — run tests
- `cargo run` — interactive REPL
- `cargo build` — build the binary
- `scripts/stress_recursion.sh` — feed deeply-nested expressions to the release binary and confirm no walker overflows the main-thread stack. Run after touching parser / annotate / validate / evaluate paths.

## Pre-commit checks

Run all three checks the CI gate (`.github/workflows/ci.yml`, job `fmt + clippy + test`) runs, before every commit — the gate rejects on any failure:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --all-targets`

## Version bumps and release tags

Commit, tag, and push are separate actions: perform each only when the user explicitly authorizes it. A version-bump request alone does not authorize a tag or push.

1. Inspect the working tree and recent commits. Keep feature/fix commits separate from the version-bump commit, and do not include unrelated changes.
2. Update `[package].version` in `Cargo.toml`. Running the Cargo checks updates the tracked `Cargo.lock`; verify that only the `vcal` package version changes there, not dependency versions.
3. Run all three pre-commit checks above before each commit. Stage the two version files explicitly and create a new commit named `Bump version to X.Y.Z`. For example (substitute the requested version):

   ```sh
   git add Cargo.toml Cargo.lock
   git commit -m "$(cat <<'EOF'
   Bump version to 0.3.1
   EOF
   )"
   ```

4. Before publication, run `git fetch origin` and inspect `git log --oneline --left-right origin/master...master`. Check that `git merge-base --is-ancestor origin/master master` succeeds. If history has diverged, stop and ask how to integrate the remote changes; do not force-push or silently merge/rebase them into the release. After any approved integration, rerun the checks and verify the release version.
5. Use an annotated tag named `vX.Y.Z`, matching existing release tags. Tag the verified release commit containing the version bump (normally the bump commit itself). Verify the target commit and check that the tag does not already exist locally or remotely; never overwrite an existing tag. For example, when the verified release commit is `HEAD`: `git tag -a v0.3.1 HEAD -m "v0.3.1"`.
6. When push is authorized, push `master` and only the requested tag: `git push origin master && git push origin v0.3.1`. Do not use `--tags`, which could publish unrelated tags. The current CI workflow runs on pushes to `master`, not tag-only pushes. Verify the remote branch/tag targets and check the CI run; a successful push does not mean CI has passed.

## Documentation boundary

- [README.md](README.md) — stable user-facing entry point. Do not edit without user input.
- [doc/scope.md](doc/scope.md) — mutable working state (current status, active scope, backlog). **Update this first** when scope changes or a task completes; collapse completed work to one-line summaries. Git history is the granular record.
- Stable implementation / spec docs under [doc/](doc/) — one topic per file:
  - [doc/repl.md](doc/repl.md) — prompt, session, exit behavior, lexical clarifications
  - [doc/expressions.md](doc/expressions.md) — width, signedness, leaf extension, base propagation
  - [doc/operators.md](doc/operators.md) — per-operator semantics
  - [doc/variables.md](doc/variables.md) — `reg`, blocking assignment, bit-select / part-select
  - [doc/non-standard.md](doc/non-standard.md) — vcal-specific divergences from LRM
  - [doc/lrm-coverage.md](doc/lrm-coverage.md) — LRM chapter coverage and grammar matrix
  - [doc/architecture.md](doc/architecture.md) — source layout and REPL entry points

Quick test for where a fact belongs: if it will still hold after the next feature ships, put it in one of the stable docs; otherwise it belongs in `doc/scope.md`.

Do not put specific dates in documents — no "confirmed on &lt;date&gt;" markers on known issues, no dated validation logs, no "as of" timestamps. State whether something is current, open, or deferred, and let git history carry the timeline (`git log`, `git blame`). Likewise prefer capability descriptions over counts that go stale on the next commit, such as exact test totals.

## Meta-rules

- Do not commit, amend, tag, or push unless the user **explicitly** says so — "commit", "tag", "push", or equivalent. Staging files for review (`git add` without a following `git commit`) is fine; the commit/push step waits for the explicit word. This avoids accidental history changes.
- Add LRM edge-case tests as new operators land.
- Most design rules derive from LRM. Where vcal diverges intentionally, the divergence is documented in [doc/non-standard.md](doc/non-standard.md) — consult it before reading the LRM.
- Do not infer scope from [doc/lrm-coverage.md](doc/lrm-coverage.md) — many checked boxes are long-term targets, not current scope. Confirm with the user before expanding beyond what [doc/scope.md](doc/scope.md) lists as active.
- Two REPL entry points: `vcal::run_interactive` (rustyline, TTY only) and `vcal::run_repl(BufRead, Write)` (piped / test). `src/main.rs` dispatches via `IsTerminal`. Both wrap stdout in `ConsoleSafeWriter` (see [doc/repl.md](doc/repl.md) → Output encoding) so non-UTF-8 formatter bytes degrade to lossy UTF-8 on a Windows console while every other destination keeps the raw byte stream. Keep both paths working.
