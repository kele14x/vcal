# Variables: reg, blocking assignment, bit/part-select

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

## Blocking assignment

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

## Bit-select and part-select

A declared reg can be sliced four ways (LRM 4.2.1 / 5.2.1 / 5.2.2):

| Syntax        | Form                       | Result width  |
|---------------|----------------------------|---------------|
| `r[expr]`     | bit-select                 | 1             |
| `r[m:l]`      | part-select                | `|m-l|+1`     |
| `r[b +: w]`   | indexed part-select up     | `w`           |
| `r[b -: w]`   | indexed part-select down   | `w`           |

The select grammar only attaches to a declared identifier — `4'b1111[0]`
does not parse, matching the LRM production
`identifier [ { [ expression ] } [ range_expression ] ]`. The unpacked
`{ dimension }` array form remains out of scope.

The same four select forms are also accepted on the left-hand side of a
blocking assignment, per LRM A.8.5 `variable_lvalue`:

| LHS form                | Meaning                                |
|-------------------------|----------------------------------------|
| `r[expr] = ...`         | write a single bit                     |
| `r[m:l] = ...`          | write a constant-bounded slice         |
| `r[b +: w] = ...`       | write an upward indexed slice          |
| `r[b -: w] = ...`       | write a downward indexed slice         |
| `{a, b[3:0], ...} = ...`| distribute bits across a concatenation |

Concatenations may nest arbitrarily (`{x, {y, z[1:0]}}`), and the leaves
are flattened left-to-right with the leftmost leaf taking the most
significant slice of the RHS. The RHS is evaluated in the
total-LHS-width context (sum of leaf widths, unsigned, leftmost leaf's
base), so the usual width / sign / base propagation rules apply.

A few semantic rules worth pinning down:

1. **Out-of-range / x-z LHS select positions are silently dropped.**
   LRM 4.2.1 says "no assignment shall be performed" for such a
   position. So `reg [3:0] r = 4'h0; r[5:2] = 4'b1111` writes only the
   in-range positions and leaves `r` as `4'b1100`; an x/z bit-select
   index drops the whole write but is not an error.
2. **Duplicate-bit writes in a concat LHS are implementation-defined.**
   IEEE 1364-2005 does not specify what happens when a target bit
   appears more than once on the LHS (`{a[0], a[0]} = ...`, or
   `{a, a[0]} = ...` where the `a` leaf and the `a[0]` leaf both
   address bit 0). vcal does not reject this; the natural right-to-left
   distribution lets the leaf closer to the MSB end of the concat write
   last, so it wins. So `reg [3:0] a = 4'h0; {a[0], a[0]} = 2'b10`
   ends with `a` as `4'b0001` — the MSB-side `a[0]` receives the RHS
   MSB (1).

   A subtler case where the duplicate-bit rule interacts with the echo
   rule: given `reg [3:0] a`, the line `{a, a[0]} = 8'b000_01xz_x`
   prints `5'b01xzx` (the echo: 8-bit RHS truncated to the 5-bit total
   LHS context per rule #4) but leaves `a` as `4'b01xz`. The two
   differ in the LSB. Distribution walks leaves right-to-left, so the
   rightmost `a[0]` leaf writes first (taking the RHS LSB `x` →
   `a[0] = x`), then the leftmost `a` leaf writes its 4 bits over
   `a[3:0]` — and *its* position 0 (the RHS bit `z`) overwrites the
   earlier `x`. So `a[0]` ends as `z`, not the `x` you'd guess from
   the echo's rightmost bit. The echo reflects "what the RHS becomes
   in the LHS context"; the reg state reflects "what survived the
   duplicate-write resolution".
3. **All-or-nothing commit.** Structural validation (direction
   mismatch, undeclared leaf, scalar-with-select, x/z in a constant
   endpoint, zero indexed width) runs before the RHS is evaluated and
   before any reg is mutated. So `{a, b_undeclared} = ...` leaves `a`
   untouched.
4. **Echo policy.** `Out[n]:` prints the RHS evaluated in the total-LHS
   context. For bare-name LHS this is bit-identical to the pre-lvalue
   behavior (reg's stored width / signedness / base); for a select LHS
   it prints the slice at the select width and the reg's base; for a
   concat LHS it prints the joined value at the sum-width with the
   leftmost leaf's base.

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
is evaluated. See [non-standard.md](non-standard.md) → "Bit-select and
part-select operands" for the rationale.

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
