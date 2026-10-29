// Integrity levels: preventExtensions < seal < freeze, what each one locks, what each one
// leaves open (accessor setters, nested objects, array length), how the predicates are
// computed, and the refusal surface (Reflect answers a boolean, strict code throws).
// Strict module code, so direct assignments are routed through Reflect.set to keep
// the observation independent of the caller's mode.
export {};

function attempt(label: string, fn: () => unknown): void {
  try {
    const r = fn();
    console.log(label + "=ok:" + String(r));
  } catch (e: any) {
    console.log(label + "=throw:" + e.constructor.name);
  }
}

// ---------- 1. preventExtensions: no additions; existing writes and deletes pass ----------
const pe: any = { x: 1 };
console.log("pe_start=" + Object.isExtensible(pe) + ",sealed=" + Object.isSealed(pe) + ",frozen=" + Object.isFrozen(pe));
Object.preventExtensions(pe);
console.log("pe_after=" + Object.isExtensible(pe));
console.log("pe_write=" + Reflect.set(pe, "x", 4) + ",x=" + pe.x);
console.log("pe_add=" + Reflect.set(pe, "y", 5) + ",has=" + Object.hasOwn(pe, "y"));
attempt("pe_add_strict", () => { pe.y = 5; return "wrote"; });
console.log("pe_delete=" + Reflect.deleteProperty(pe, "x") + ",keys=" + Object.keys(pe).join("|"));
console.log("pe_readd_after_delete=" + Reflect.set(pe, "x", 7) + ",has=" + Object.hasOwn(pe, "x"));
console.log("pe_sealed_now=" + Object.isSealed(pe) + ",frozen=" + Object.isFrozen(pe));
console.log("pe_returns_same=" + (Object.preventExtensions(pe) === pe));

// a non-extensible object that still has a configurable property is NOT sealed
const pe2: any = Object.preventExtensions({ keep: 1 });
console.log("pe2_sealed=" + Object.isSealed(pe2) + ",frozen=" + Object.isFrozen(pe2));
// an empty non-extensible object counts as both sealed and frozen
const pe3: any = Object.preventExtensions({});
console.log("pe3_empty=" + Object.isSealed(pe3) + "," + Object.isFrozen(pe3));

// ---------- 2. seal: no additions, no deletes, every property non-configurable; writes pass ----------
const sl: any = { x: 1, y: 2 };
Object.seal(sl);
console.log("sl_flags=" + Object.isSealed(sl) + ",frozen=" + Object.isFrozen(sl) + ",ext=" + Object.isExtensible(sl));
console.log("sl_write=" + Reflect.set(sl, "x", 10) + ",x=" + sl.x);
console.log("sl_add=" + Reflect.set(sl, "z", 1) + ",has=" + ("z" in sl));
console.log("sl_delete=" + Reflect.deleteProperty(sl, "x") + ",keys=" + Object.keys(sl).join("|"));
attempt("sl_delete_strict", () => delete sl.y);
const slx = Object.getOwnPropertyDescriptor(sl, "x") as any;
console.log("sl_desc=w=" + slx.writable + ",c=" + slx.configurable);
console.log("sl_redefine=" + Reflect.defineProperty(sl, "x", { value: 3, writable: false }) + ",x=" + sl.x);
console.log("sl_redefine_ok=" + Reflect.defineProperty(sl, "x", { value: 3 }) + ",x=" + sl.x);

// seal is shallow: nested objects are untouched
const slNested: any = Object.seal({ inner: { n: 1 } });
slNested.inner.n = 2;
console.log("sl_nested=" + slNested.inner.n + ",inner_sealed=" + Object.isSealed(slNested.inner));

// a sealed object with an accessor: the setter still runs
let sideEffect = "none";
const slAcc: any = {};
Object.defineProperty(slAcc, "acc", { get() { return sideEffect; }, set(v: string) { sideEffect = "set:" + v; }, configurable: true, enumerable: true });
Object.seal(slAcc);
console.log("sl_acc_write=" + Reflect.set(slAcc, "acc", "A") + ",side=" + sideEffect);

// ---------- 3. freeze: data properties read-only and non-configurable; accessors keep their setters ----------
const fz: any = { a: 1, b: "x" };
Object.freeze(fz);
console.log("fz_flags=" + Object.isFrozen(fz) + ",sealed=" + Object.isSealed(fz) + ",ext=" + Object.isExtensible(fz));
console.log("fz_write=" + Reflect.set(fz, "a", 10) + ",a=" + fz.a);
console.log("fz_add=" + Reflect.set(fz, "c", 1) + ",has=" + ("c" in fz));
console.log("fz_delete=" + Reflect.deleteProperty(fz, "b") + ",json=" + JSON.stringify(fz));
const fza = Object.getOwnPropertyDescriptor(fz, "a") as any;
console.log("fz_desc=w=" + fza.writable + ",c=" + fza.configurable + ",e=" + fza.enumerable);
console.log("fz_keys=" + Object.keys(fz).join("|"));

// freeze is shallow: a nested object stays writable
const fzNested: any = Object.freeze({ top: 2, inner: { n: 1 } });
fzNested.inner.n = 7;
console.log("fz_nested=" + fzNested.inner.n + ",inner_frozen=" + Object.isFrozen(fzNested.inner));

// an accessor is not writable-cleared by freeze: its setter still runs
let counter = 0;
const fzAcc: any = {};
Object.defineProperty(fzAcc, "acc", { get() { return "acc:" + counter; }, set(v: number) { counter = v; }, enumerable: true, configurable: true });
Object.freeze(fzAcc);
const acd = Object.getOwnPropertyDescriptor(fzAcc, "acc") as any;
console.log("fz_acc_desc=get:" + typeof acd.get + ",set:" + typeof acd.set + ",c=" + acd.configurable + ",hasW=" + ("writable" in acd));
console.log("fz_acc_write=" + Reflect.set(fzAcc, "acc", 5) + ",read=" + fzAcc.acc);
console.log("fz_acc_frozen=" + Object.isFrozen(fzAcc) + ",sealed=" + Object.isSealed(fzAcc));

// an object with ONLY accessors that are non-configurable is frozen once non-extensible
const onlyAcc: any = {};
Object.defineProperty(onlyAcc, "x", { get() { return 1; }, set() { /* noop */ }, configurable: false });
Object.preventExtensions(onlyAcc);
console.log("only_acc_frozen=" + Object.isFrozen(onlyAcc) + ",sealed=" + Object.isSealed(onlyAcc));

// freezing a primitive is the identity; the predicates answer true for primitives
console.log("freeze_prim=" + (Object.freeze(7 as any) as any) + ",isFrozen_prim=" + Object.isFrozen(7) + "," + Object.isFrozen("abc"));
console.log("seal_prim=" + (Object.seal(null as any) as any) + "," + Object.isSealed(null as any));

// freeze an empty typed array: fine. A non-empty one has indexed elements that cannot be made read-only
const emptyTA = new Uint8Array(0);
console.log("freeze_empty_ta=" + Object.isFrozen(Object.freeze(emptyTA)));
attempt("freeze_full_ta", () => Object.freeze(new Uint8Array([1])));

// ---------- 4. Arrays under each level ----------
const frozenArr: any = Object.freeze([1, 2]);
console.log("fa_push=" + (() => { try { frozenArr.push(3); return "ok"; } catch (e: any) { return "throw:" + e.constructor.name; } })());
console.log("fa_write=" + Reflect.set(frozenArr, "0", 9) + ",v=" + frozenArr[0] + ",len=" + frozenArr.length);
console.log("fa_len_writable=" + (Object.getOwnPropertyDescriptor(frozenArr, "length") as any).writable);

const sealedArr: any = Object.seal([1, 2]);
console.log("sa_write=" + Reflect.set(sealedArr, "0", 9) + ",json=" + JSON.stringify(sealedArr));
console.log("sa_len_writable=" + (Object.getOwnPropertyDescriptor(sealedArr, "length") as any).writable);
console.log("sa_frozen=" + Object.isFrozen(sealedArr));

const peArr: any = [1, 2, 3];
Object.preventExtensions(peArr);
console.log("pea_push=" + (() => { try { peArr.push(4); return "ok"; } catch (e: any) { return "throw:" + e.constructor.name; } })());
peArr.length = 1;
console.log("pea_shrunk=" + JSON.stringify(peArr) + ",len=" + peArr.length);
console.log("pea_regrow=" + Reflect.set(peArr, "1", 9) + ",json=" + JSON.stringify(peArr));
console.log("pea_sealed=" + Object.isSealed(peArr) + ",frozen=" + Object.isFrozen(peArr));

// ---------- 5. Refusals in the same result shape across the three levels ----------
const levels: any[] = [
  Object.preventExtensions({ v: 1 }),
  Object.seal({ v: 1 }),
  Object.freeze({ v: 1 }),
];
levels.forEach((o, i) => {
  const name = ["pe", "sealed", "frozen"][i];
  console.log(name + "_add=" + Reflect.set(o, "n", 1) + ",delete=" + Reflect.deleteProperty(o, "v") + ",write=" + Reflect.set(o, "v", 2) + ",v=" + o.v);
});

// Object.defineProperty on a sealed or frozen object refuses changes, and the refusal throws from Object.*
const dfz: any = Object.freeze({ k: "fixed" });
attempt("dfz_redefine_value", () => Object.defineProperty(dfz, "k", { value: "other" }));
attempt("dfz_redefine_same", () => Object.defineProperty(dfz, "k", { value: "fixed" }));
attempt("dfz_add", () => Object.defineProperty(dfz, "n", { value: 1 }));
console.log("dfz_final=" + dfz.k + ",keys=" + Object.keys(dfz).join("|"));

// Object.isFrozen / isSealed are derived from the properties, not from a stored flag
const derived: any = { q: 1 };
Object.preventExtensions(derived);
console.log("derived_sealed_before=" + Object.isSealed(derived));
Object.defineProperty(derived, "q", { configurable: false });
console.log("derived_sealed_after=" + Object.isSealed(derived) + ",frozen=" + Object.isFrozen(derived));
Object.defineProperty(derived, "q", { writable: false });
console.log("derived_frozen=" + Object.isFrozen(derived));
