# What a property access costs, and why three of four bench lines stayed slow

Measured 2026-11-22 on `../rts/bench/property_access.ts`, release build,
3 000 000 iterations per line. The four lines do exactly the same arithmetic
and differ only in where the loop state lives, so a difference between two
lines is the cost of one thing.

| line | before this session | after | node |
| --- | --- | --- | --- |
| `local` | 2058 ms | **30 ms** | 49 ms |
| `capturada` | 6310 ms | **431 ms** | 78 ms |
| `propriedade` | 5891 ms | 3876 ms | 52 ms |
| `duas hops` | 7344 ms | 5289 ms | 52 ms |

## The one cause behind all four

Nothing here was a missing optimization in the JIT. The JIT and OSR machinery
is built and does engage. What switched it off in every slow line is the same
thing: **an opcode or a type in the loop that Tier 1 cannot take**, which keeps
the loop in the interpreter however hot it gets.

`crates/zun_codegen/src/eligible.rs` gates on an allowlist. A method is
eligible only if every local, parameter and the return is a JIT value type, and
only if every opcode in the region is on the list. The OSR entry takes *every*
local as a parameter, so one `any` local disqualifies the hot loop region as
well as the whole method.

## What was fixed, and what each fix was worth

1. **A typed module binding keeps its type.** A top-level `const N: number`
   read by a function came back as `any`, which made the receiving local `any`,
   which disqualified the method. `1097 -> 31 ms` on an isolated loop.
2. **A literal module constant folds at its use sites.** A binding nothing
   assigns to, initialised from a literal, *is* that literal. `1989 -> 29 ms`,
   and it is what took `local` to 30 ms.
3. **ToNumber of a typed number operand emits nothing.** A mixed-type numeric
   operator boxed *both* operands and called `numberCoerceStrict` on each, so
   `o.s * 1664525` boxed a constant and called a native per iteration. Removing
   it: `capturada` 5486 -> 4595, `propriedade` 4864 -> 4063 ms.
4. **A typed module binding lives in a static field.** `globalGet`/`globalSet`
   are calls keyed by a string, and the translator lowers neither, so any loop
   touching a module binding stayed interpreted. Such a binding now gets a
   static field of a synthetic module type -- the CLR's `<Module>` -- read with
   `ldsfld` and written with `stsfld`, both already supported. `capturada`
   4595 -> 657 ms, and the loop body became pure f64:
   `ldsfld; mul.f64; add.f64; rem.f64; stsfld`.
5. **A static field access no longer allocates its slot key.** Static storage
   is one dynamic object whose string keys are the slots, and the key was
   `format!("{owner}:{field}")` per access -- a heap string on every read and
   write of every static field of every class, in both tiers. `capturada`
   657 -> 431 ms.

## What is left, in the order it is worth doing

### `capturada`, 431 ms against node's 78

The loop is compiled and the body is pure f64. What remains is that a static
field is still **found by name**: `ldsfld` lowers to a native call to
`zun_jit_static_get(owner, field, kind)`, which hashes the statics object's
address and then hashes the slot key. Two hash lookups and a call where the CLR
has a load from a fixed address.

The fix is to stop keying statics by string. A FieldDef RID is already globally
unique within a module, so it is a dense index; statics could be a flat store
indexed by it, and `ldsfld` could then lower to a plain `load` at a constant
address with no call at all. Two things make this more than a rename:

- GC. The current design piggybacks on a dynamic object precisely so the values
  are scanned and relocated for free. A flat store must be added to root
  scanning and to the relocation pass.
- "Unset" must stay distinguishable from `undefined`, because `static_get`
  answering `None` is what makes `static_inherited_read` run. An array of `Any`
  has no hole, so a sentinel is needed or inheritance of statics breaks.

### `propriedade` 3876 ms and `duas hops` 5289 ms, against node's 52

These two are not JIT'd at all. `const o = { s: 1 }` is a `DynamicObject`, and
its accesses emit `lddyn`/`stdyn` -- opcodes that appear **nowhere** in
`crates/zun_codegen` (a grep for `LoadDynamic` there returns no hits). They are
not on the eligibility allowlist, so the loop region is rejected and the whole
loop runs in Tier 0. 3876 ms over 3 000 000 iterations is ~1.3 us per
iteration, which is interpreter dispatch, not hashing.

There are two ways forward and they are not alternatives so much as a sequence.

**(a) Lower `lddyn`/`stdyn` at Tier 1.** This is what makes the loop compile at
all, and it is the smaller change, but it is not a plain helper call: the
interpreter's `op_load_dynamic` can run managed code (a getter, a proxy trap,
`valueOf`) and can redirect control flow for an exception -- which is what
`requires_exception_bridge` exists to keep out of JIT frames. Without
de-optimization (not implemented; zun does not speculate yet, see
`docs/design/module-jit-osr.md` slice 5) a compiled site that meets a getter
has nowhere to fall back to. So this needs either a re-entrancy path from a JIT
frame into the interpreter, or de-opt first.

**(b) Give an object literal a fixed layout.** This is the Zun answer and the
one the concept asks for: `{ s: 1 }` has a known shape, never gains a key and
never escapes, so it should be an anonymous type with a typed `s: number`
field, reached by `ldfld`/`stfld` -- which the translator already lowers. It
also fixes `duas hops` by the same stroke. It is the larger change: synthesized
TypeDefs, assignability against structural annotations, and a decision for
every literal that *does* escape or get a dynamic key.

Note what the bench already observed and this analysis confirms: a captured
variable *is* a property, and `duas hops` costing the same as one hop means the
per-site cost does not dominate. Both facts point at (b) rather than at making
each individual access cheaper.

### Not a performance item, but the highest return for running real code

`../rts/bench/analytic.ts` (2452 lines) does not compile: 6 sema errors, 5 of
one family -- `cannot add Float(F64) and Int(...)`. There is no mixed float/int
arithmetic rule. The 169-fixture conformance suite is written in a style the
checker accepts, so no fixture can see this class of defect.
