# Variables: reg / integer / real, blocking assignment, bit/part-select

vcal recognizes three variable kinds at the top of a decl: `reg`,
`integer`, and `real` (LRM A.2.1.3 / 4.8). `time` is out of scope.

```text
reg [signed] [range] name { , name }
integer name [= init] { , name [= init] }
real    name [= init] { , name [= init] }
```

- A reg declaration has no strong display base. vcal uses binary as a
  fallback for an unassigned reg, so `reg [7:0] a` reads back as
  `8'bxxxxxxxx`, but the first whole-reg integer init or blocking
  assignment resolves the display base from the RHS expression:
  `reg [7:0] a = 8'h00; a` reads back as `8'h00`. Signedness still comes
  only from the declaration, so `reg signed [7:0] a = 8'hff` reads back
  as `8'shff`.
- An unsized decl is 1 bit (`reg a` → `1'bx`).
- Range halves are constant integer expressions evaluated in the current
  session at decl time; they must be non-negative and free of x/z bits.
  A reversed range (`reg [0:7] a`) yields the same width as its normal
  form per LRM 4.8.
- Multiple names can share one decl: `reg [3:0] a, b, c`.
- Redeclaring an existing identifier in the same session replaces the
  prior binding — the REPL is single-scope and a redecl is the user's way
  of resetting a reg's metadata, so the new decl wipes the old width,
  signedness, display base, and bit pattern. The freshly redeclared reg
  starts at all `x` like any other new reg.
- A fresh reg is initialized to all `x`. The decl statement prints a
  bare blank line — the same convention assignments, `$finish`, and
  `$stop` use for non-value statements (see [repl.md](repl.md)).

## `integer`

An `integer` decl is a fixed-shape `reg` (LRM 4.8):

- Signed 32-bit, decimal display base. `integer i` reads back as `32'sdx`
  before any assignment; after `i = 5` it reads as `32'sd5`.
- The `signed` qualifier and a packed `[range]` are rejected at parse
  time — the element shape is fixed. Use `reg signed [31:0] i` if you
  want a hand-rolled equivalent that can carry a non-decimal base or a
  different width.
- A single unpacked dimension is accepted (`integer a [0:3]`), and the
  array follows the rules in [Unpacked arrays](#unpacked-arrays) below
  — every element shares the fixed integer element template.
  Multi-dimensional arrays remain out of scope.
- Bit- and part-selects are the same as on a vector reg: `i[0]`, `i[3:0]`,
  `i[b +: w]`, `i[b -: w]`. The select inherits the integer's decimal
  base.
- Per-name inits use the same `name = constant_expression` form as `reg`;
  multi-name decls (`integer i = 1, j = 2, k`) follow the same
  left-to-right rule (`j` can reference `i`). Array names cannot carry
  an init expression.

## `real`

A `real` decl is an IEEE 754 binary64 slot (LRM 4.8 / 3.5.2):

- No width, signedness, or display base — `real r` reads back as `0.0`
  by default (LRM 4.8: reals are zero-initialized, not x-initialized).
- The `signed` qualifier and packed `[range]` are rejected at parse
  time, as is any bit-select on a scalar real reg.
- A single unpacked dimension is accepted (`real r [0:3]`), giving a
  1-D array of f64 slots. The element-select rules are restricted
  (real elements have no bits to slice) — see [Unpacked arrays](#unpacked-arrays).
  Multi-dimensional arrays remain out of scope.
- Init / RHS may be a real-typed or integer-typed expression. Integer
  RHS promotes to f64 via §5.1.7 / §3.5.3 (x/z bits → 0). NaN / ±∞ are
  preserved in a `real` LHS, but assigning them to an integer LHS fills
  with x bits (matching `$rtoi`'s "no defined integer" handling).
- Per-name inits and the staged all-or-nothing commit work the same as
  `reg` and `integer`. Array names cannot carry an init expression.

## Blocking assignment

Blocking assignment `name = expression` is a top-level statement, not an
expression (LRM A.6.2), so it does not nest inside larger expressions.
The LHS reg's width, signedness, and current display base flow into the
RHS via the standard §5.6 context rules, then the resulting bits replace
the reg's bits. If the LHS is a whole `reg` whose display base is still
weak, an integer RHS resolves it; later whole-reg assignments preserve
the resolved base. Real-typed RHS expressions do not resolve a weak
display base. Bit-select, part-select, array-element, and concat-lvalue
writes update bits without resolving the whole reg's display base.

A real-typed RHS goes through an implicit real→integer conversion per
LRM §3.5.3 (round to nearest, ties away from zero — the same rule
`$itor`'s internal real→int step uses); NaN / ±∞ have no integer image
and surface as the lvalue filled with x bits at its declared width. The
assignment itself prints a blank line per [repl.md](repl.md); reference
the reg on the next line (or as a trailing expression on the same line)
to display its new value.

An identifier reference resolves to the reg's current bits and then
participates in the surrounding expression like any other primary — its
`(width, signed, base)` propagates per §5.5 (so e.g. an 8-bit reg whose
base resolved to hex makes a leftmost `+` result render in hex).
Referencing an undeclared name is an error.

## Bit-select and part-select

A declared reg can be sliced four ways (LRM 4.2.1 / 5.2.1 / 5.2.2):

| Syntax        | Form                       | Result width  |
|---------------|----------------------------|---------------|
| `r[expr]`     | bit-select                 | 1             |
| `r[m:l]`      | part-select                | `\|m-l\|+1`   |
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
   MSB (1). The right-to-left walk also explains a subtler case: with
   `reg [3:0] a` and `{a, a[0]} = 8'b000_01xz_x` (RHS truncated to the
   5-bit LHS context as `5'b01xzx`), the rightmost `a[0]` leaf writes
   first (taking the RHS LSB `x` → `a[0] = x`), then the leftmost `a`
   leaf writes its 4 bits over `a[3:0]`. *Its* position 0 (the RHS
   bit `z`) overwrites the earlier `x`, so `a` ends as `4'b01xz` —
   not what a left-to-right walk would give.
3. **All-or-nothing commit.** Structural validation (direction
   mismatch, undeclared leaf, scalar-with-select, x/z in a constant
   endpoint, zero indexed width) runs before the RHS is evaluated and
   before any reg is mutated. So `{a, b_undeclared} = ...` leaves `a`
   untouched.

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

## Unpacked arrays

A `reg`, `integer`, or `real` decl may add one unpacked dimension to
declare a 1-D array (LRM 4.9 / A.2.2.1):

```text
reg [packed_range] name [unpacked_range] { , name [unpacked_range] }
integer            name [unpacked_range] { , name [unpacked_range] }
real               name [unpacked_range] { , name [unpacked_range] }
```

For example, `reg [3:0] a [0:15]` declares 16 elements, each a 4-bit
unsigned vector; `integer a [0:3]` declares 4 elements, each a signed
32-bit decimal slot; `real r [0:3]` declares 4 IEEE 754 binary64 slots.
The packed range governs each element's shape (fixed for `integer`,
absent for `real`); the unpacked range governs how many elements exist
and how they are addressed. Both endpoints follow the same
constant-integer rule the packed range uses (no x/z, evaluated against
the current session).

- Multi-dimensional arrays, packed-array-of-array forms, and array
  initializers are out of scope — only the single unpacked dimension
  shown above is accepted, regardless of the element type.
- Every unpacked array is limited to 65,536 elements so the per-element
  storage and staging metadata remain bounded. Vector arrays are additionally
  limited to 16,777,216 total payload bits across all elements.
- An array decl cannot carry an init expression: `reg [3:0] a [0:7] = ...`
  / `integer a [0:3] = 5` / `real r [0:3] = 1.5` are all rejected. Each
  element starts at the element type's default: all-`x` of the packed
  shape for `reg` and `integer`, `0.0` for `real`.
- The unpacked range may be reversed (`reg [3:0] a [15:0]`) or use
  negative endpoints (`reg [3:0] a [-2:1]`); the resolved index
  mapping uses `internal = |src - lsb_unpacked|`, the same formula
  the packed-range mapping uses.
- A `real` array element is f64-typed, so it has no bits to address.
  `r[i]` is the only legal element select on a real array — `r[m:l]`,
  `r[b +: w]`, `r[b -: w]`, and the chained `r[i][...]` form are all
  rejected. Reads / writes flow through the real pipeline (RHS integers
  promote to f64 via §3.5.3); an OOB or x/z index reads as `0.0` and
  drops the write per LRM 4.2.1. A real-array element cannot appear
  inside an lvalue concat (concats are bit-based).

The array name on its own is unreadable — `a` (where `a` is an array)
errors with "array `a` cannot be used as a value". The only way to
read or write through an array is via an element-select on the outer
unpacked dim:

| Form              | Meaning                                          |
|-------------------|--------------------------------------------------|
| `a[i]`            | the whole packed element at unpacked index `i`   |
| `a[i][n]`         | bit `n` of that element                          |
| `a[i][m:l]`       | part-select of that element                      |
| `a[i][b +: w]`    | upward indexed part-select of that element       |
| `a[i][b -: w]`    | downward indexed part-select of that element     |

The outer select must be a `Bit` form (`a[i]`) — `a[m:l]`, `a[b +: w]`,
`a[b -: w]` are all rejected with "part-select on array `a` is illegal".
The inner select, when present, runs against the chosen element's
packed range and follows the same rules vector-reg selects do
(direction match, width > 0, real-typed endpoints rejected, etc.).

Element-select reads (RHS):

- `a[i]` returns the chosen element in its packed shape, or an all-`x`
  value of that shape if `i` is x/z or out of range (LRM 4.2.1 + 4.9).
- `a[i][...]` first resolves the element (x/z or OOB index → all-`x`
  element fallback), then evaluates the inner select against that
  element. So `a[1'bx][0]` returns `1'bx`, and an inner-select against
  an OOB element returns x bits at every position.
- Bit-/part-select on a scalar-array element (`reg a [0:7]`) is
  rejected — there are no bits to address.

Element-select writes (LHS):

- `a[i] = expr` writes the whole element in element-shape context
  (width / signed from the packed range; reg-array elements currently use
  the same binary fallback display base as a fresh scalar `reg`).
- `a[i][n] = expr`, `a[i][m:l] = expr`, and the indexed forms write
  only the named positions of the chosen element; other positions are
  preserved. The RHS evaluates in the *inner select's* shape (width
  set by the form, unsigned per LRM 4.7, base inherited from the
  element).
- An x/z or OOB outer index drops the entire write — no element is
  mutated — but the echo still prints the RHS in the lvalue's shape so
  the calculator output is consistent with the vector-reg
  "x/z-index drops the write" case. The same per-position drop rule
  the vector-reg path uses applies to the inner select: `a[0][5:2]`
  on a 4-bit element drops bits 4 and 5 (OOB), writes bits 3:2.
- Array-element leaves may appear inside a concat LHS: `{b, a[0][2:0],
  a[1]} = 11'b...` distributes the RHS bit stream right-to-left across
  the leaves just like a vector-only concat would. A bare array name as
  a concat leaf is still rejected (no way to address all elements at
  once).
- All-or-nothing commit still holds: any structural error on any leaf
  aborts the whole assignment before any reg is mutated.
