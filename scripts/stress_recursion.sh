#!/usr/bin/env bash
#
# Stress-test vcal against deeply-nested expression input.
#
# Each case generates an N-deep expression with Python and pipes it to
# the vcal binary. The harness reports a failure when the binary exits
# with SIGABRT (rc=134) or prints "stack overflow" — both signal that a
# recursive walker overflowed the main-thread stack. Any non-zero exit
# other than 1 (clean diagnostic) is also treated as failure.
#
# Two phases:
#   1. parse-only  (cheap, exercises every parsing path)
#   2. full eval   (parse + annotate + validate + evaluate)
#
# Two depth knobs:
#   DEPTH=N        depth used for parse-only cases       (default 100000)
#   EVAL_DEPTH=N   depth used for full-pipeline cases    (default 50000)
#   VCAL=path      vcal binary to test                   (default ./target/release/vcal)
#   STRESS_TIMEOUT=seconds   per-case wall-clock cap     (default 60)
#
# Usage:
#   cargo build --release && scripts/stress_recursion.sh
#   DEPTH=200000 scripts/stress_recursion.sh
#   VCAL=./target/debug/vcal scripts/stress_recursion.sh
#
# Exits 0 if every case passes; non-zero if any case fails.
#
set -u

VCAL="${VCAL:-./target/release/vcal}"
DEPTH="${DEPTH:-100000}"
EVAL_DEPTH="${EVAL_DEPTH:-50000}"
STRESS_TIMEOUT="${STRESS_TIMEOUT:-60}"

if [[ ! -x "$VCAL" ]]; then
    echo "vcal binary not found or not executable: $VCAL" >&2
    echo "build it first: cargo build --release" >&2
    exit 2
fi

# `timeout` is GNU coreutils; on macOS it's `gtimeout` from coreutils
# (brew). If neither is present, run without a wall-clock cap — a
# recursive crash is fast (microseconds), so the cap is only protection
# against a runaway non-crashing case.
TIMEOUT_BIN=""
if command -v gtimeout >/dev/null 2>&1; then
    TIMEOUT_BIN="gtimeout"
elif command -v timeout >/dev/null 2>&1; then
    TIMEOUT_BIN="timeout"
fi

ERR_FILE="$(mktemp -t vcal_stress_err.XXXXXX)"
trap 'rm -f "$ERR_FILE"' EXIT

pass=0
fail=0
declare -a failures

run_case() {
    local name="$1"
    local genscript="$2"
    shift 2

    local input
    input="$(python3 -c "$genscript")"

    # Wrap the pipeline in a subshell with stderr suppressed so bash's
    # own pipeline-status diagnostics ("Done" / "Abort trap: 6") for a
    # SIGABRT'd child don't pollute the harness output. The vcal binary's
    # actual stderr is captured via $ERR_FILE inside the subshell.
    local rc=0
    if [[ -n "$TIMEOUT_BIN" ]]; then
        ( printf '%s' "$input" | "$TIMEOUT_BIN" "$STRESS_TIMEOUT" "$VCAL" "$@" >/dev/null 2>"$ERR_FILE" ) 2>/dev/null || rc=$?
    else
        ( printf '%s' "$input" | "$VCAL" "$@" >/dev/null 2>"$ERR_FILE" ) 2>/dev/null || rc=$?
    fi
    local err
    err="$(cat "$ERR_FILE")"

    # SIGABRT (134) or any "stack overflow" message → recursive crash.
    # Plain rc=1 means a clean diagnostic was printed — that's a pass for
    # this harness; we only care that no walker overflowed.
    # rc=124 from timeout means the case ran longer than STRESS_TIMEOUT.
    if [[ $rc -eq 134 ]] || [[ "$err" == *"stack overflow"* ]]; then
        printf 'FAIL  %-50s rc=%d (stack overflow)\n' "$name" "$rc"
        failures+=("$name")
        fail=$((fail + 1))
        return
    fi
    if [[ $rc -eq 124 ]]; then
        printf 'FAIL  %-50s rc=124 (timed out after %ss)\n' "$name" "$STRESS_TIMEOUT"
        failures+=("$name")
        fail=$((fail + 1))
        return
    fi
    if [[ $rc -ne 0 && $rc -ne 1 ]]; then
        printf 'FAIL  %-50s rc=%d %s\n' "$name" "$rc" \
            "$(printf '%s' "$err" | head -c 120)"
        failures+=("$name")
        fail=$((fail + 1))
        return
    fi

    printf 'ok    %-50s rc=%d\n' "$name" "$rc"
    pass=$((pass + 1))
}

echo "vcal:        $VCAL"
echo "DEPTH:       $DEPTH (parse-only)"
echo "EVAL_DEPTH:  $EVAL_DEPTH (full pipeline)"
echo "timeout:     ${TIMEOUT_BIN:-<none>} ${STRESS_TIMEOUT}s"
echo

# ============================================================
# Parser-only shapes (--parse-only): exercise every parse path
# without paying for evaluation. DEPTH should sit comfortably
# above the previously-recorded crash thresholds (parser used to
# overflow at ~7K for braces / system functions, ~2K for raw
# parens before the iterative-parser series).
# ============================================================
echo "=== parse-only at depth=$DEPTH ==="

run_case "parens                    (((..1..)))"        "n=$DEPTH; print('('*n + '1' + ')'*n)" --parse-only
run_case "concat-nested             {{{..1'b1..}}}"     "n=$DEPTH; print('{'*n + \"1'b1\" + '}'*n)" --parse-only
run_case "replication-nested        {1{{1{..}}}}"       "n=$DEPTH; print('{1{'*n + \"1'b1\" + '}}'*n)" --parse-only
run_case "signed-nested             \$signed(\$signed(..))"     "n=$DEPTH; print('\$signed('*n + '1' + ')'*n)" --parse-only
run_case "unsigned-nested           \$unsigned(\$unsigned(..))" "n=$DEPTH; print('\$unsigned('*n + '1' + ')'*n)" --parse-only
run_case "pow-rhs-nested            \$pow(2,\$pow(2,..))"       "n=$DEPTH; print('\$pow(2,'*n + '1' + ')'*n)" --parse-only
run_case "clog2-nested              \$clog2(\$clog2(..))"       "n=$DEPTH; print('\$clog2('*n + '1' + ')'*n)" --parse-only
run_case "sqrt-nested               \$sqrt(\$sqrt(..))"         "n=$DEPTH; print('\$sqrt('*n + '1.0' + ')'*n)" --parse-only
run_case "rtoi-itor-nested          \$rtoi(\$itor(..))"         "n=$DEPTH; print(('\$rtoi(\$itor('*(n//2)) + '1' + (')'*((n//2)*2)))" --parse-only
run_case "unary-tilde               ~~~..~1"            "n=$DEPTH; print('~'*n + '1')" --parse-only
run_case "unary-bang                !!!..!1"            "n=$DEPTH; print('!'*n + '1')" --parse-only
run_case "unary-plus                +++..+1"            "n=$DEPTH; print('+'*n + '1')" --parse-only
run_case "unary-minus               ---..-1"            "n=$DEPTH; print('-'*n + '1')" --parse-only
run_case "unary-redand              &&&..&1"            "n=$DEPTH; print('&'*n + '1')" --parse-only
run_case "unary-redor               |||..|1"            "n=$DEPTH; print('|'*n + '1')" --parse-only
run_case "unary-redxor              ^^^..^1"            "n=$DEPTH; print('^'*n + '1')" --parse-only
run_case "binary-add                1+1+..+1"           "n=$DEPTH; print('1' + '+1'*n)" --parse-only
run_case "binary-mul                1*1*..*1"           "n=$DEPTH; print('1' + '*1'*n)" --parse-only
run_case "binary-shift              1<<1<<..<<1"        "n=$DEPTH; print('1' + '<<1'*n)" --parse-only
run_case "binary-rel                1<1<..<1"           "n=$DEPTH; print('1' + '<1'*n)" --parse-only
run_case "binary-eq                 1==1==..==1"        "n=$DEPTH; print('1' + '==1'*n)" --parse-only
run_case "binary-and                1&&1&&..&&1"        "n=$DEPTH; print('1' + '&&1'*n)" --parse-only
run_case "binary-or                 1||1||..||1"        "n=$DEPTH; print('1' + '||1'*n)" --parse-only
run_case "binary-bitand             1&1&..&1"           "n=$DEPTH; print('1' + '&1'*n)" --parse-only
run_case "binary-bitxor             1^1^..^1"           "n=$DEPTH; print('1' + '^1'*n)" --parse-only
run_case "binary-power              1**1**..**1"        "n=$DEPTH; print('1' + '**1'*n)" --parse-only
run_case "ternary-right-assoc       1?1:1?1:..:0"       "n=$DEPTH; print('1?1:'*n + '0')" --parse-only
run_case "ternary-cond-deep         (..1..)?1:0"        "n=$DEPTH; print('('*n + '1' + ')'*n + '?1:0')" --parse-only
run_case "wide-flat-concat          {1,1,..,1}"         "n=$DEPTH; print('{1' + ',1'*n + '}')" --parse-only
run_case "concat-of-deep-add        {1+1+..+1}"         "n=$DEPTH; print('{1' + '+1'*n + '}')" --parse-only
run_case "rep-count-deep-add        {(1+1+..){1'b1}}"   "n=$DEPTH; print('{(1' + '+1'*n + '){1\\'b1}}')" --parse-only
run_case "select-index-deep         a[1+1+..+1]"        "n=$DEPTH; print('a[1' + '+1'*n + ']')" --parse-only
run_case "select-range-deep         a[1+1+..:0]"        "n=$DEPTH; print('a[1' + '+1'*n + ':0]')" --parse-only
run_case "mixed-paren-concat        ({{(((..))}})"      "n=$DEPTH; m=n//4; print('({' + '({'*m + '1' + '})'*m + '})')" --parse-only
run_case "mixed-signed-concat       \$signed({\$signed({..})})"  "n=$DEPTH; m=n//2; print('\$signed({'*m + '1' + '})'*m)" --parse-only
run_case "mixed-unary-paren         ~(((..)))~"         "n=$DEPTH; print('~(' * n + '1' + ')'*n)" --parse-only
run_case "mixed-unary-binary        ~~~(1+1+..+1)"      "n=$DEPTH; print('~'*n + '(1' + '+1'*n + ')')" --parse-only
run_case "deep-real-add             1.0+1.0+..+1.0"    "n=$DEPTH; print('1.0' + '+1.0'*n)" --parse-only
run_case "atan2-arity2-nested       \$atan2(1.0,\$atan2(..))"   "n=$DEPTH; print('\$atan2(1.0,'*n + '1.0' + ')'*n)" --parse-only
run_case "ln-arity1-nested          \$ln(\$ln(..))"             "n=$DEPTH; print('\$ln('*n + '1.0' + ')'*n)" --parse-only

# ============================================================
# Full pipeline (parse + annotate + validate + evaluate). Use a
# smaller depth here because evaluation is O(N) per shape — these
# cases would otherwise dominate runtime. We only need to confirm
# that no walker along the chain overflows.
# ============================================================
echo
echo "=== full eval at depth=$EVAL_DEPTH ==="

run_case "eval parens               (((..1..)))"        "n=$EVAL_DEPTH; print('('*n + '1' + ')'*n)"
run_case "eval concat-nested        {{{..1'b1..}}}"     "n=$EVAL_DEPTH; print('{'*n + \"1'b1\" + '}'*n)"
run_case "eval replication-nested   {1{{1{..}}}}"       "n=$EVAL_DEPTH; print('{1{'*n + \"1'b1\" + '}}'*n)"
run_case "eval signed-nested        \$signed(\$signed(..))"     "n=$EVAL_DEPTH; print('\$signed('*n + '1' + ')'*n)"
run_case "eval unsigned-nested      \$unsigned(\$unsigned(..))" "n=$EVAL_DEPTH; print('\$unsigned('*n + '1' + ')'*n)"
run_case "eval pow-rhs              \$pow(2,\$pow(2,..))"       "n=$EVAL_DEPTH; print('\$pow(2,'*n + '1' + ')'*n)"
run_case "eval integer-power-rhs    1**(1**(..))"               "n=$EVAL_DEPTH; print('1**('*n + '1' + ')'*n)"
run_case "eval clog2-nested         \$clog2(\$clog2(..))"       "n=$EVAL_DEPTH; print('\$clog2('*n + '4' + ')'*n)"
run_case "eval rtoi-itor            \$rtoi(\$itor(..))"         "n=$EVAL_DEPTH; print(('\$rtoi(\$itor('*(n//2)) + '1' + (')'*((n//2)*2)))"
run_case "eval ln-arity1            \$ln(\$ln(..))"             "n=$EVAL_DEPTH; print('\$ln('*n + '1.0' + ')'*n)"
run_case "eval atan2-arity2         \$atan2(1.0,\$atan2(..))"   "n=$EVAL_DEPTH; print('\$atan2(1.0,'*n + '1.0' + ')'*n)"
run_case "eval bitstoreal-realtobits"                                       "n=$EVAL_DEPTH; print(('\$bitstoreal(\$realtobits('*(n//2)) + '1.0' + (')'*((n//2)*2)))"
run_case "eval unary-tilde          ~~~..~1"            "n=$EVAL_DEPTH; print('~'*n + '1')"
run_case "eval unary-bang           !!!..!1"            "n=$EVAL_DEPTH; print('!'*n + '1')"
run_case "eval unary-plus           +++..+1"            "n=$EVAL_DEPTH; print('+'*n + '1')"
run_case "eval unary-minus          ---..-1"            "n=$EVAL_DEPTH; print('-'*n + '1')"
run_case "eval unary-redand         &&&..&1"            "n=$EVAL_DEPTH; print('&'*n + '1')"
run_case "eval unary-redor          |||..|1"            "n=$EVAL_DEPTH; print('|'*n + '1')"
run_case "eval unary-redxor         ^^^..^1"            "n=$EVAL_DEPTH; print('^'*n + '1')"
run_case "eval binary-add           1+1+..+1"           "n=$EVAL_DEPTH; print('1' + '+1'*n)"
run_case "eval binary-mul           1*1*..*1"           "n=$EVAL_DEPTH; print('1' + '*1'*n)"
run_case "eval binary-rel           1<1<..<1"           "n=$EVAL_DEPTH; print('1' + '<1'*n)"
run_case "eval binary-eq            1==1==..==1"        "n=$EVAL_DEPTH; print('1' + '==1'*n)"
run_case "eval binary-and           1&&1&&..&&1"        "n=$EVAL_DEPTH; print('1' + '&&1'*n)"
run_case "eval binary-or            1||1||..||1"        "n=$EVAL_DEPTH; print('1' + '||1'*n)"
run_case "eval binary-bitand        1&1&..&1"           "n=$EVAL_DEPTH; print('1' + '&1'*n)"
run_case "eval binary-bitxor        1^1^..^1"           "n=$EVAL_DEPTH; print('1' + '^1'*n)"
run_case "eval binary-shift         1<<1<<..<<1"        "n=$EVAL_DEPTH; print('1' + '<<1'*n)"
run_case "eval ternary-rassoc       1?1:1?1:..:0"       "n=$EVAL_DEPTH; print('1?1:'*n + '0')"
run_case "eval ternary-cond-deep    (..1..)?1:0"        "n=$EVAL_DEPTH; print('('*n + '1' + ')'*n + '?1:0')"
run_case "eval wide-concat          {1,1,..,1}"         "n=$EVAL_DEPTH; print('{1' + ',1'*n + '}')"
run_case "eval concat-of-deep-add   {1+1+..+1}"         "n=$EVAL_DEPTH; print('{1' + '+1'*n + '}')"
run_case "eval real-add             1.0+1.0+..+1.0"    "n=$EVAL_DEPTH; print('1.0' + '+1.0'*n)"
run_case "eval power-deep-rhs       2**(1+1+..+1)"      "n=$EVAL_DEPTH; print('2**(1' + '+1'*n + ')')"
run_case "eval power-deep-rhs-rel   2**(1<1<..<1)"     "n=$EVAL_DEPTH; print('2**(1' + '<1'*n + ')')"
run_case "eval power-deep-rhs-bitand 2**(1&1&..&1)"     "n=$EVAL_DEPTH; print('2**(1' + '&1'*n + ')')"
run_case "eval power-deep-rhs-and   2**(1&&1&&..&&1)"   "n=$EVAL_DEPTH; print('2**(1' + '&&1'*n + ')')"
run_case "eval power-deep-rhs-shift 2**(1<<1<<..<<1)"   "n=$EVAL_DEPTH; print('2**(1' + '<<1'*n + ')')"
run_case "eval rep-count-deep-add   {(1+1+..){1'b1}}"   "n=$EVAL_DEPTH; print('{(1' + '+1'*n + \"){1'b1}}\")"
run_case "eval pow-of-deep-add      \$pow(2,1+1+..+1)"  "n=$EVAL_DEPTH; print('\$pow(2,1' + '+1'*n + ')')"
run_case "eval itor-of-deep-add     \$itor(1+1+..+1)"   "n=$EVAL_DEPTH; print('\$itor(1' + '+1'*n + ')')"
run_case "eval signed-of-concat     \$signed({1+1+..+1})"      "n=$EVAL_DEPTH; print('\$signed({1' + '+1'*n + '})')"
run_case "eval mixed-binary-ops     1+1*1-1&1|1^1.."   "n=$EVAL_DEPTH; ops=['+','*','-','&','|','^']; print('1' + ''.join(ops[i%len(ops)]+'1' for i in range(n)))"

# ============================================================
# LValue concat: `{{{..a}}} = 1` exercises `expression_to_lvalue`
# (parser-side Expr→LValue), `lvalue_meta` (LRM 5.6 LHS context
# derivation), and `flatten_lvalue_leaves` (bit-distribution walker).
# All three are now iterative; `impl Drop for LValue` mirrors
# `impl Drop for Expr` so the resulting deep `Box<LValue>` chain drops
# without overflow.
# ============================================================
run_case "eval lvalue-concat        {{{..a}}} = 1"     "n=$EVAL_DEPTH; print('reg a;'); print('{'*n + 'a' + '}'*n + '=1;')"
run_case "eval lvalue-wide-concat   {a,a,..,a} = ..."  "n=$EVAL_DEPTH; print('reg a;'); print('{a' + ',a'*n + '}=' + str(n+1) + \"'b\" + '0'*(n+1) + ';')"

# ============================================================
# Top-level system task wrapped in parens. After the system-
# identifier parser unification, the lib driver walks parens
# iteratively via `unwrap_grouped` to spot a top-level `$finish`
# / `$stop` and exit — no more recursive `top_level_task_name`.
# ============================================================
run_case "eval paren-wrapped-finish ((( \$finish )))"  "n=$EVAL_DEPTH; print('(' * n + '\$finish' + ')' * n)"

# ============================================================
# Bug #2: MAX_BIT_WIDTH cap rejects huge widths before allocation.
# Each case used to hang the kernel for minutes (10 TB / 2 GB / 4 GB
# virtual allocations being page-faulted into anonymous memory);
# they now exit in milliseconds with a clean diagnostic. The
# at-cap case checks the inclusive-accept boundary still works.
# ============================================================
run_case "eval huge-literal-width    9999999999999'd1"       "print(\"9999999999999'd1\")"
run_case "eval huge-replication      {2147483647{1'b1}}"     "print(\"{2147483647{1'b1}}\")"
run_case "eval huge-indexed-select   r[3 +: 4294967296]"     "print('reg [3:0] r;'); print('r[3 +: 4294967296];')"
run_case "eval huge-concat-sum       {a, a} a=16M+1 bits"    "print('reg [16777215:0] a;'); print('{a, a};')"
run_case "eval at-cap-literal-width  16777216'd1"            "print(\"16777216'd1\")"
run_case "eval at-cap-concat-sum     {a, a} a=8M bits"       "print('reg [8388607:0] a;'); print('{a, a};')"

# ============================================================
# Summary
# ============================================================
echo
echo "----- summary -----"
echo "passed: $pass"
echo "failed: $fail"
if (( fail > 0 )); then
    echo "failures:"
    for f in "${failures[@]}"; do echo "  - $f"; done
    exit 1
fi
