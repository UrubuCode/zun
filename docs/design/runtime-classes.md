# Run-time classes

Status: implemented for the shapes listed under "What is lowered"; the gaps are
listed under "Not covered".

## The problem

Zun compiles a class to **one static TypeDef**: fixed layout, vtable dispatch,
and a class value that is an immediate descriptor (`USER_CLASS_FLAG | rid`).
Every evaluation of the same class text therefore yields the same value:

```ts
function make(tag: string): any { return class { label: string = tag; }; }
const A = make("a"), B = make("b");
A !== B                        // was false
A.prototype !== B.prototype    // was false
new A().label                  // threw: `tag` is not a local of any method
```

Three capabilities were missing, and they are one capability:

1. **identity per evaluation** -- its own constructor and its own prototype;
2. **a class environment** -- members close over the enclosing function's
   locals (`tag` above), exactly as a function expression does;
3. **a base known only at run time** -- `class extends Base` with `Base` a
   parameter or a variable (a mixin), where `super` must reach whatever the
   value is.

A class in JavaScript is a closure. A static TypeDef cannot be one.

## Why not a per-evaluation descriptor

The existing descriptor model was examined first, because it would have kept
typed layouts:

- A class value is an immediate token, so it cannot carry identity. Its record
  form (`{ __classDesc }`, `classBox`) compares **by token** on purpose, so that
  `x.constructor === C` holds.
- An instance can already carry a prototype other than its TypeDef's (the
  instance bag's link, used by `Reflect.construct(.., newTarget)`), so identity
  alone could be layered: a record with its own `prototype` over a shared
  template TypeDef.
- It stops there. A member compiled as a typed method has **no environment
  slot**: the format has no notion of a method closed over a class
  environment, so captures (2) cannot be delivered, and neither can the home
  object `super` needs when one mixin is applied twice in a chain. And with a
  run-time base (3) the instance cannot be the template's typed object at all:
  it is whatever the base constructs.

Making that work means inventing a closure-carrying method kind in the format,
or a TypeDef at run time. The first is a format change with interpreter and JIT
halves; the second breaks the rule that the compiler and the VM share only the
`.zm` module.

## The design: lower the class to the dynamic model, on purpose

A class that is a run-time value is **not hoisted**. The front end
(`crates/zun_syntax/src/oxc/runtime_class.rs`) lowers it in place to the form
the dynamic object model already supports -- a constructor function, a
prototype object, and members that are closures:

```ts
(() => {                                   // fresh bindings per evaluation
  const B = Base;                          // heritage, evaluated first
  let C = undefined;
  const I = function () { this.field = init; };        // field initializers
  C = function (...) { ... };                            // the constructor
  __zunClassBegin(C, "Name", true, B);
  __zunClassMethod(C.prototype, "m", function () { ... });
  __zunClassStatic(C, "count", 0);
  return C;
})()
```

Everything that follows is ordinary IL: closures capture the factory's locals
through the existing capture analysis, two evaluations are two function values
with two prototypes, and `instanceof`, `constructor`, `getPrototypeOf`,
`Object.keys` and descriptors read the real objects. Nothing crosses the
compile/run line except the module, no opcode is added, and no type is created
at run time. This is the "explicit, costly exception" of the architecture, used
where the class genuinely is dynamic.

The prelude (`crates/zun_prelude/builtin/runtime_class.zun`) carries the class
semantics the closure form does not give by itself:

| Helper | Role |
|---|---|
| `__zunClassBegin` | heritage validation (`Class extends value ... is not a constructor or null`), `prototype` over the base's, constructor chained to the base, `constructor` / `name`, read-only `prototype` |
| `__zunClassMethod` / `Getter` / `Setter` / `Static` | members with the descriptors a class gives them |
| `__zunSuperCall` | `super(...)`: `Reflect.construct(base, args, newTarget)`, then this class's field initializers |
| `__zunNewTarget` | `new.target`, read off the object `new` allocated |
| `__zunDerivedThis` | the ReferenceError for a derived constructor that never called `super` |
| `__zunClassCallCheck` | a class constructor refuses a plain call |

### `super`, `this` and `new.target`

- `super.m(args)` is `Object.getPrototypeOf(C.prototype).m.call(this, args)`
  (`Object.getPrototypeOf(C)` in a static member); `super.x` is a
  `Reflect.get` from the same home object with `this` as the receiver. The
  home object is the *captured* class binding, so two applications of one
  mixin in a chain each reach their own base.
- In a **derived** constructor `this` is the object `super(...)` returned, not
  the one `new` pre-allocated. It lives in a one-element cell (`[value]`)
  that arrows capture by value; every `this` in the body converts to `cell[0]`.
- `new.target` is recovered from the pre-allocated object (its prototype's
  `constructor`), which is what carries the most derived prototype down a
  `super(...)` chain: `Reflect.construct(base, args, newTarget)`.

### What is lowered

Decided at the syntax boundary (`wants_runtime_class`):

- a class whose **base is a run-time value**: any heritage expression other
  than a class name, `null`, a class expression, or a call to a function
  declared `: typeof Name`; and a heritage identifier that is a parameter or
  local of an enclosing function, or a module binding that is not a class;
- a **class expression used as a value inside a function** (returned, passed,
  stored) -- evaluated once per call;
- a **class in any position inside a function whose body mentions a binding
  of an enclosing function** -- a declaration, a class bound by a declarator
  (`const C = class {}`), a class constructed in place. See "Capture" below.

A class declaration, a class bound by a declarator and a class constructed in
place stay static otherwise: a local class that captures nothing keeps its
typed layout. A lowered class declaration is bound like a `let`.

### Capture: a method sees the enclosing function's locals, by reference

```ts
function mut(): any { let n = 1; class K { read() { return n } } const k = new K(); n = 99; return k.read() }  // 99
function shared(): any[] { let c = 0; class C { inc() { return ++c } } return [new C(), () => c] }
const [o, peek] = shared(); o.inc(); o.inc(); peek()                                                            // 2
```

The sharing is observable, so a copy taken when the class is evaluated is
wrong. Nothing new was built for it: the lowered members are function values,
so the ordinary capture analysis gives them the enclosing environment
(`MakeClosure` over the `NewDynamic` environment, `StoreDynamic` captures) --
the same cell an arrow of that scope holds. There is no capture slot on the
class or on its instances, so nothing shows up in `Object.keys`,
`getOwnPropertyNames`, `in` or a descriptor.

What had to be decided is *routing*: which classes take that path
(`crates/zun_syntax/src/oxc/class_capture.rs`).

- **The bindings of a function** are collected from its AST when the function
  is entered: parameters (destructured and rest included), `var` / `let` /
  `const` at any block depth, nested function declarations and catch
  parameters -- not descending into nested functions. A binding initialized
  by a class expression is a static class name, not a run-time binding, until
  that class is itself lowered.
- **The class body is scanned as tokens** for one of those names, discarding
  the positions where a name is certainly not a variable reference: after `.`
  (not `...`), as a `key:` in an object literal, parameter list or member
  declaration, and as a member name at class-body level outside a field
  initializer. Strings and comments are skipped; template holes are scanned.
- The scan errs **towards lowering**. A false positive (a method parameter
  that shadows an enclosing local, say) costs the typed layout of one class;
  a false negative would be a `ReferenceError`. The front end has no scope
  analysis of its own to do better with; the checker cannot make the decision
  because by then the class is already hoisted.
- **Lowering propagates.** A lowered class name is a run-time binding of its
  function, so a class that mentions it or extends it is lowered too. The
  sibling classes of a block are settled to a fixpoint before any is
  converted (`predeclare_forward_classes`), so the order they are written in
  does not matter.

Two consequences of a lowered class being a `let` rather than a program-wide
name are handled at the syntax boundary:

- **A closure written before the declaration.** JavaScript binds a class from
  the top of its block; a closure only captures a binding that exists when it
  is created. When an earlier statement of the block mentions the class, the
  binding is pre-declared (`let K: any = undefined`) at the top of the block
  and the declaration becomes an assignment -- the pattern forward-referenced
  function declarations already use.
- **A static class of the same name elsewhere.** The checker resolves
  `new Name()`, `x instanceof Name` and a type annotation `: Name` against the
  program-wide class table *before* local bindings, ignoring where the class
  is visible. With a static `class K` in any other function, `new K()` on the
  lowered `K` constructed the other class, silently. So a name that lexically
  resolves to a lowered class (`names_runtime_class`, innermost function
  first, shadowed by a nearer static class or ordinary binding) converts
  `new K()` to a dynamic `new` on the binding, `x instanceof K` to a test
  against the binding's value (`K as any`), and erases `: K` to `any`.

A class the closure form cannot express -- decorators, parameter properties,
`accessor` fields -- **keeps the static path**, with its
existing diagnostics (including "`super` is not supported in a class whose base
class is a run-time value").

### What the runtime needed

The dynamic model held up almost entirely; five gaps in it were closed, each
one a real JavaScript rule that an ordinary constructor function also needs:

- a function's `prototype.constructor` was never defined (a function value
  does not box into `Any`, so the definition silently did nothing);
- `instance.constructor` ignored the prototype chain and answered `Object`;
- a store through an accessor on the prototype chain created an own property
  instead of calling the setter;
- statics were not inherited through a function's `[[Prototype]]`
  (`Object.setPrototypeOf(F, Base)`), for a function or a class parent;
- `toString` / `valueOf` / `Symbol.toPrimitive` on the prototype chain were
  ignored by ToPrimitive (`String(instance)`, `` `${instance}` ``, `instance + 1`).

Plus three call-site rules: a function read out of an array is constructible
(`new list[i]()`), `new C()` inside a closure reaches an enclosing local `C`,
and a static method is called with the function itself as `this` (it was the
function's property bag, so `this === C` was false and `new this()` did not
compile).

## Not covered

Each of these fails loudly unless marked otherwise.

- **A capturing class that cannot be lowered still captures nothing.**
  `function f(p) { class K { constructor(public x: number) {} m() { return p } } }`
  throws `p is not defined`: decorators, parameter properties and `accessor`
  fields keep the static path (see above), and a static method has no
  environment.
- **A function value returned by a method called on a dynamic receiver is
  lost.** `const o: any = { fn() { return function () {} } }; typeof o.fn()`
  answers `object`, and calling the result throws `null is not a function`.
  It predates run-time classes (the object literal alone shows it) but every
  lowered class is a dynamic receiver, so `class K { handler() { return () =>
  p } }` and a method that returns a class (`return Inner`) answer
  `undefined`. Returning the function inside an object or an array works.
  This is the most damaging open gap for lowered classes; it belongs in the
  dynamic call path, not in the lowering.
- **A static member named `call` (or `apply` / `bind`)** on a run-time class
  is shadowed by `Function.prototype`'s: `C.call()` invokes the constructor
  and throws `Class constructor C cannot be invoked without 'new'`.
- **No temporal dead zone.** A pre-declared class binding holds `undefined`
  until its declaration runs, so `new K()` written *before* a capturing
  `class K` in straight-line code throws a `TypeError` (not a constructor)
  where JavaScript throws a `ReferenceError`. A static local class does not
  throw at all there. Both wait for TDZ support.
- **A binding declared after the class.** `class K { m() { return later } }
  const later = 3;` throws `later is not defined`. It is not specific to
  classes -- `const f = () => later; const later = 3; f()` fails the same
  way -- a closure only captures bindings that exist when it is created.
- **Block granularity.** Bindings and lowered names are tracked per function,
  not per block: two classes of one name in sibling blocks of one function,
  one capturing and one not, are both treated as the lowered one by `new`.
- **The checker's own shadowing bug is only routed around for lowered
  classes.** Any other local named like a class declared anywhere in the
  program is still ignored by `new`: `const K: any = function () {...};
  new K()` constructs a static `class K` from another function. The fix
  belongs in `check_new` (a visible local must win over the class table).
  Routing *every* local through the dynamic `new` at the syntax boundary was
  measured and rejected: it fixes that program but breaks
  `tests/misc-platform/platform-classes-and-helpers.ts` (a local function
  used as a constructor takes a different path through a dynamic `new`).
- **A `Proxy` as a base** (`class X extends new Proxy(C, {})`) throws
  `Class extends value does not have valid prototype property undefined`: a
  proxy over a class value answers `undefined` for every member read, and
  `Reflect.construct(proxy, ...)` rejects it. Both predate this work.
- **Decorators, parameter properties and `accessor` fields in a run-time
  class**: the class keeps the static path (see above). Parameter properties
  are assignments at the head of the lowered constructor; `accessor` is a
  private field plus a getter/setter pair, so it follows from the private
  lowering below. Neither is done.
- **`obj?.#x`** in a run-time class is a compile error (as it is in a static
  one).
- **Private error messages on the static path differ from node's** (a static
  private read on a subclass says `undefined is not a function`); the lowered
  path uses node's wording.
- **Typed fields of a static base reject values of the wrong type.**
  `new Mixed("Bob", null)` over `class User { age: number }` aborts in the
  typed field store (`value.rs:157`). The same program without any run-time
  class aborts identically; it is the open `as_number` family.
- **Silent, reflection only**: `Function.prototype.toString` of a run-time
  class does not start with `class`, and an anonymous class does not take its
  name from the property key it is stored under.
- **Silent**: a field initializer is an assignment, not a definition, so a
  setter of the same name on the base's prototype is called. A class
  constructor called with a receiver that looks like a fresh object
  (`C.call({})`) is not refused.


## Private names of a run-time class

```ts
class Outer {
  #n = "outer";
  makeInner(): any {
    const self = this;
    class Inner {
      #m = "inner";
      both() { return this.#m + "/" + self.#n; }     // captures `self`
      static innerBrand(o: any) { return #m in o; }
      static outerBrand(o: any) { return #n in o; }
    }
    return Inner;
  }
}
```

`Inner` captures, so it must be a closure; it also declares `#m`. Two designs
were considered.

1. **An environment slot on a static class.** Keep `Inner` typed and give its
   methods a hidden environment. This is the "closure-carrying method kind"
   the first section rejects: a format change with interpreter and JIT
   halves, a hidden slot that reflection must be taught to skip, and static
   methods (no `this`) still need a per-evaluation class value. It also
   leaves identity per evaluation unsolved.
2. **Private names in the closure form** (implemented). A private name is
   per-evaluation state exactly like a capture, so it lowers to bindings of
   the class factory. No format change, no opcode, no hidden slot.

### The lowered form

`crates/zun_syntax/src/oxc/runtime_private.rs` gives each private name of a
lowered class a *slot*; the prelude helpers (`__zunPriv*` in
`builtin/runtime_class.zun`) do the brand check and the access.

| Member | Where it lives | Brand |
|---|---|---|
| instance field `#x` | its own `WeakMap`, instance -> value | membership in that map |
| instance method / getter / setter | a functions bag (`{}`) of closures | one `WeakMap` per class, filled before the fields |
| static field | a statics bag (`{}`) | identity with the class |
| static method / getter / setter | the functions bag | identity with the class |

```ts
(() => {
  const W = __zunPrivStore();                        // #m
  const I = function () { __zunPrivDefine(W, this, "inner", "m"); };
  C = function () { ...; I.call(this); };
  __zunClassMethod(C.prototype, "both", function () {
    return __zunPrivRead(0, W, this, W, "m", C) + "/" + self.#n;   // `#n` is Outer's
  });
  __zunClassStatic... innerBrand: function (o) { return __zunPrivIn(0, W, o, "m"); }
})()
```

- **References.** `o.#x` is `__zunPrivRead`, `o.#x = v` (and the compound,
  logical and `++` forms, through the existing once-only decomposition of the
  target) is `__zunPrivWrite`, `o.#x(args)` is `__zunPrivCall` (read, then
  call on `o`), `#x in o` is `__zunPrivIn`. All take the same leading
  arguments, `(kind, guard, o, store, key, class)`; `kind` selects the row of
  the table above and its low bit says "static".
- **Brands.** `#x in o` answers membership in the store, so it follows the
  *object*: still true after `Object.setPrototypeOf(o, null)`, false for
  `Object.create(C.prototype)`, false for an instance of another evaluation
  of the same class text, and a `TypeError` for a primitive. The static
  path's `hasPrivateBrand` (the type chain) is not involved and is unchanged
  for static classes. A static private is branded by `o === C`, so a
  subclass receiver is refused.
- **Errors.** A wrong receiver gets node's `TypeError`: `Cannot read private
  member #x from an object whose class did not declare it` (fields),
  `Receiver must be an instance of class C` / `Receiver must be class C`
  (methods and accessors), `'#x' was defined without a getter|setter`,
  `Private method '#x' is not writable`, `Cannot initialize #x twice on the
  same object` (a base constructor returning an already-branded object).
- **Reflection.** Nothing is stored on the instance, the class or the
  prototype, so `getOwnPropertyNames`, `keys`, `getOwnPropertySymbols`,
  `Reflect.ownKeys`, descriptors, `in`, `for..in`, `JSON.stringify`, spread
  and `Object.assign` have nothing to find. A frozen instance still takes
  private writes, as in JavaScript.
- **Order.** The stores are declared before any member closure is created (a
  closure only captures a binding that exists) and never reassigned. Private
  methods are installed ahead of every other member, so a static initializer
  can call them. An instance is branded for its private methods before its
  fields are defined, in declaration order, inside the existing
  field-initializer function -- so in a derived class the brand lands on
  whatever `super(...)` returned.

### Lexical scope

A private name resolves to the innermost class that declares it. The front
end keeps a stack of frames mirroring the class nesting of the node being
converted: a lowered class pushes its slots, a static class pushes its names
as *shadows*. A reference whose innermost frame is a slot is lowered; any
other reference (the `#n` of the static `Outer` above, or a name a nested
static class redeclares) is left untouched for the checker, which already
resolves an outer static class's private name inside a nested function.

### What changed in routing

`lowering_supported` no longer excludes `#private` members, so a class with
them is lowered under the same three rules as any other: a run-time base, a
class expression used as a value inside a function, or a capture. A class
with private members that meets none of them is still a static class with a
typed layout and type-chain brands.
