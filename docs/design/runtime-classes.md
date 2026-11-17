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
  stored) -- evaluated once per call.

A class declaration, a class bound by a declarator (`const C = class {}`) and a
class constructed in place stay static unless their base is a run-time value.
A lowered class declaration is bound like a `let`.

A class the closure form cannot express -- `#private` members, decorators,
parameter properties, `accessor` fields -- **keeps the static path**, with its
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

- **A class declared (or bound by a declarator) inside a function still
  captures nothing.** `function f(p) { class K { m() { return p } } ... }`
  throws `p is not defined`, as before. Only class *expressions used as
  values* are lowered; widening the rule would move typed local classes onto
  the dynamic path, which is a performance decision, not a correctness one.
- **A `Proxy` as a base** (`class X extends new Proxy(C, {})`) throws
  `Class extends value does not have valid prototype property undefined`: a
  proxy over a class value answers `undefined` for every member read, and
  `Reflect.construct(proxy, ...)` rejects it. Both predate this work.
- **`#private` members and decorators in a run-time class**: the class keeps
  the static path (see above).
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
