// `this` in strict code: a method's receiver comes from the call site, a
// detached method loses it, an arrow captures it lexically and ignores every
// call/apply/bind, class bodies never box a primitive receiver, and extracted
// accessors, static methods and shadowing behave as the spec says. An arrow's
// lack of its own `arguments` and `new.target` is covered here too.
export {};

const show = (label: string, value: unknown): void => {
  console.log(label + "=" + JSON.stringify(value));
};

function tryOf(fn: () => unknown): string {
  try {
    return "ok:" + String(fn());
  } catch (e) {
    return "threw:" + (e as any).name;
  }
}

// ---------------------------------------------------------------------------
// 1. Methods versus arrow fields on a class
// ---------------------------------------------------------------------------
class Widget {
  tag = "widget";
  read(): string { return this === undefined ? "lost" : (this as any).tag; }
  readArrow = (): string => (this === undefined ? "lost" : this.tag);
  kindOfThis(): string { return this === undefined ? "undefined" : typeof this; }
  isSame(other: any): boolean { return (this as any) === other; }
  static staticRead(): string { return this === undefined ? "lost" : (this as any).name; }
}
const w = new Widget();
show("method", w.read());
show("arrow_field", w.readArrow());

const detachedMethod = w.read;
const detachedArrow = w.readArrow;
show("detached_method", tryOf(() => detachedMethod()));
show("detached_arrow", detachedArrow());

function invoke(fn: () => string): string { return fn(); }
show("callback_method", tryOf(() => invoke(w.read)));
show("callback_arrow", invoke(w.readArrow));
show("callback_bound", invoke(w.read.bind(w)));
show("callback_wrapped", invoke(() => w.read()));

const other: any = { tag: "other" };
show("method_call", detachedMethod.call(other));
show("method_apply", detachedMethod.apply(other));
show("method_bind", detachedMethod.bind(other)());
show("arrow_call_ignored", detachedArrow.call(other as any));
show("arrow_apply_ignored", detachedArrow.apply(other as any));
show("arrow_bind_ignored", detachedArrow.bind(other as any)());

// Strict class code: a primitive receiver arrives unboxed, null stays null.
show("this_undefined", w.kindOfThis.call(undefined));
show("this_null", w.kindOfThis.call(null));
show("this_number", w.kindOfThis.call(42));
show("this_string", w.kindOfThis.call("txt"));
show("this_boolean", w.kindOfThis.call(true));
show("this_bigint", w.kindOfThis.call(10n));
show("this_object", w.kindOfThis.call({}));
show("identity_number", w.isSame.call(7, 7));
show("identity_object", w.isSame.call(other, other));

// Static methods: the class is the receiver when called on it.
show("static_on_class", Widget.staticRead());
const detachedStatic = Widget.staticRead;
show("static_detached", tryOf(() => detachedStatic()));
show("static_rebound", detachedStatic.call(Widget));

// Each instance has its OWN arrow field; the method lives once on the prototype.
const w2 = new Widget();
w2.tag = "second";
show("arrow_per_instance", w.readArrow !== w2.readArrow);
show("method_shared", w.read === w2.read);
show("method_on_prototype", Object.prototype.hasOwnProperty.call(Widget.prototype, "read"));
show("arrow_on_instance", Object.prototype.hasOwnProperty.call(w, "readArrow"));
show("arrow_on_prototype", Object.prototype.hasOwnProperty.call(Widget.prototype, "readArrow"));
show("second_arrow", w2.readArrow());

// The arrow keeps its instance even when copied onto a foreign object.
const moved: any = { tag: "moved", readArrow: w.readArrow };
show("moved_arrow", moved.readArrow());

const md = Object.getOwnPropertyDescriptor(Widget.prototype, "read") as any;
const ad = Object.getOwnPropertyDescriptor(w, "readArrow") as any;
show("method_attrs", [md.enumerable, md.writable]);
show("arrow_field_attrs", [ad.enumerable, ad.writable]);

class Sub extends Widget {
  tag = "sub";
  describe(): string { return this.read() + "/" + this.readArrow(); }
}
const s = new Sub();
show("subclass_describe", s.describe());
show("subclass_detached_method", invoke(s.read));
show("subclass_arrow_detached", invoke(s.readArrow));
show("subclass_method_inherited", s.read === Widget.prototype.read);

class Later {
  tag = "later";
  defer(): () => string { return () => this.tag; }
}
const deferred = new Later().defer();
show("deferred_arrow", deferred());
show("deferred_call_ignored", deferred.call({ tag: "nope" } as any));

// ---------------------------------------------------------------------------
// 2. Object-literal methods versus arrows
// ---------------------------------------------------------------------------
const withArrow: any = { label: "literal", read: w.readArrow };
const withMethod: any = {
  label: "literal",
  read(suffix: string): string { return this.label + suffix; },
};
show("arrow_as_member_ignores_owner", withArrow.read());
show("method_as_member", withMethod.read("-method"));
const takenOut = withMethod.read;
show("method_taken_out_rebound", takenOut.call({ label: "given" }, "-rebound"));
show("method_taken_out_bound", takenOut.bind(withMethod)("-bound"));

// A method and its wrapper: the wrapper re-reads the owner on every call.
let live: any = { tag: "first", show(): string { return this.tag; } };
const wrapper = (): string => live.show();
show("wrapper_first", wrapper());
live = { tag: "second", show(): string { return this.tag; } };
show("wrapper_after_swap", wrapper());

// Getters are not extracted by reading them: the read runs the getter.
class Computed {
  get twice(): number { return 21 * 2; }
}
const computed = new Computed();
show("getter_read", computed.twice);
const getterFn: any = (Object.getOwnPropertyDescriptor(Computed.prototype, "twice") as any).get;
show("getter_extracted", [typeof getterFn, getterFn.name]);
show("getter_called_with_receiver", getterFn.call(computed));

// A method assigned onto an instance shadows the prototype's; deleting uncovers it.
class Account {
  label: string;
  balance: number;
  boundShow: () => string;
  constructor(label: string, balance: number) {
    this.label = label;
    this.balance = balance;
    this.boundShow = (): string => this.label + ":" + this.balance;
  }
  show(): string { return this.label + ":" + this.balance; }
  add(n: number): string {
    this.balance += n;
    return this.show();
  }
}
const acct = new Account("A", 10);
const extracted = acct.show;
show("as_method", acct.show());
show("extracted", tryOf(() => extracted()));
show("same_function_as_prototype", extracted === Account.prototype.show);
show("not_own_property", Object.prototype.hasOwnProperty.call(acct, "show"));
show("via_call", extracted.call(acct));
show("via_apply", extracted.apply(acct, []));
show("via_bind", extracted.bind(acct)());
show("reflect_apply_receiver", Reflect.apply(Account.prototype.show, acct, []));

const arrowExtracted = acct.boundShow;
show("field_arrow_extracted", arrowExtracted());
show("field_arrow_is_own", Object.prototype.hasOwnProperty.call(acct, "boundShow"));
show("field_arrow_per_instance", acct.boundShow !== new Account("B", 0).boundShow);

const boundOnce = acct.show.bind(acct);
const boundTwice = acct.show.bind(acct);
show("bind_identity", [boundOnce !== boundTwice, boundOnce() === boundTwice()]);

const boundAdd = acct.add.bind(acct);
show("bound_mutation", [boundAdd(5), boundAdd(5)]);
show("instance_after_bound", acct.show());
show("foreign_receiver", extracted.call({ label: "foreign", balance: 99 }));

const adopted: any = { label: "adopted", balance: 3, show: Account.prototype.show };
show("adopted_method", adopted.show());
show("adopted_same_fn", adopted.show === Account.prototype.show);

const shadowed = new Account("S", 1);
(shadowed as any).show = function (): string { return "own-version"; };
show("shadowed", shadowed.show());
delete (shadowed as any).show;
show("after_delete", shadowed.show());

show("bound_after_swap_value", boundOnce());
show("bound_meta", [boundOnce.name, boundOnce.length, acct.show.name, acct.show.length]);

// Methods taken from the prototype and used as callbacks.
const accounts = [new Account("X", 1), new Account("Y", 2)];
show("callback_bound_map", accounts.map((a) => a.show()));
show("callback_extracted_map", accounts.map((a) => tryOf(() => (a.show as any)())));
show("callback_uncurried", accounts.map(Account.prototype.show.call.bind(Account.prototype.show)));

// thisArg of a built-in iteration supplies the receiver; an arrow ignores it.
const collected: string[] = [];
[1, 2].forEach(function (this: any, n: number): void {
  collected.push(this.label + n);
}, acct);
show("this_arg", collected);
const arrowCollected: string[] = [];
[1, 2].forEach((n: number): void => { arrowCollected.push(String(n)); }, acct);
show("arrow_ignores_thisarg", arrowCollected);

// ---------------------------------------------------------------------------
// 3. Arrows have no `this`, `arguments` or `new.target` of their own
// ---------------------------------------------------------------------------
class Host {
  label: string;
  constructor(label: string) { this.label = label; }
  makeReader(): any { return (suffix: string): string => this.label + suffix; }
  makeNested(): any { return (): any => (): string => this.label + "/nested"; }
  countsOuter(a: number = 0): any {
    return (): string => arguments.length + ":" + String((arguments as any)[0]);
  }
  newTargetProbe(): any { return (): string => String(new.target); }
}
const host = new Host("H");
const reader = host.makeReader();
show("reader_plain", reader("-plain"));
show("reader_call", reader.call(other, "-call"));
show("reader_apply", reader.apply(other, ["-apply"]));
show("reader_bind", reader.bind(other)("-bind"));
show("reader_bind_partial", reader.bind(other, "-partial")());
show("reader_double_bind", reader.bind(other).bind({ label: "THIRD" })("-double"));
show("nested_arrows", host.makeNested()()());
host.label = "H2";
show("arrow_reads_live_this", reader("-live"));
host.label = "H";
const readerB = host.makeReader();
show("two_arrows_distinct", [reader !== readerB, reader("-x") === readerB("-x")]);
show("arrow_has_prototype", Object.prototype.hasOwnProperty.call(reader, "prototype"));
show("arrow_own_names", Object.getOwnPropertyNames(reader).sort());
show("new_arrow", tryOf(() => new (reader as any)("x")));
show("new_bound_arrow", tryOf(() => new (reader.bind(other) as any)("x")));
show("new_target_in_method_arrow", host.newTargetProbe()());
show("arguments_from_arrow", host.countsOuter(5, 6 as any)());

function outerWithArguments(a: number = 0): any {
  const inner = (...rest: any[]): string =>
    "outer=" + arguments.length + " inner_rest=" + rest.length;
  return inner;
}
show("arrow_rest_vs_arguments", outerWithArguments(1, 2 as any)(9, 9, 9));

const twoParams = (a: number, b: number): number => a + b;
show("arrow_length_bound", [twoParams.length, twoParams.bind(null, 1).length]);
show("arrow_typeof", [typeof twoParams, twoParams instanceof Function]);

// An arrow used as a callback needs no binding, and a default parameter that is
// an arrow closes over the same `this`.
class Adder {
  total = 0;
  addAll(values: number[]): number {
    values.forEach((v) => { this.total += v; });
    return this.total;
  }
  addAllLoose(values: number[]): string {
    return tryOf(() => {
      values.forEach(function (this: any, v: number): void { this.total += v; });
      return this.total;
    });
  }
}
show("arrow_callback", new Adder().addAll([1, 2, 3]));
show("function_callback_loose", new Adder().addAllLoose([1, 2, 3]));

class Defaults {
  label = "D";
  run(make: any = (): string => this.label + "-default"): string { return make(); }
}
show("default_arrow", new Defaults().run());
show("default_supplied", new Defaults().run(() => "supplied"));

// Class-field and literal arrows never receive `this` from a caller.
class Holder {
  value = 1;
  get(): () => number { return () => this.value; }
}
const hGet = new Holder().get();
show("arrow_from_method_detached", hGet.call({ value: 99 } as any));
