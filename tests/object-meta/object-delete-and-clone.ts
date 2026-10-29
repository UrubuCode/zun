// Deletion semantics (configurable vs refused, operator vs Reflect, proxy invariant) and
// the faithful-clone idiom Object.create(getPrototypeOf(o), getOwnPropertyDescriptors(o))
// compared with spread and Object.assign, which flatten descriptors into data slots.
// Strict module code: refusals are observed through Reflect.deleteProperty (boolean).
export {};

// ---------- 1. The delete operator on deletable things ----------
const o1: any = { a: 1, b: 2 };
console.log("op_own=" + (delete o1.a) + ",keys=" + Object.keys(o1).join("|"));
console.log("op_missing=" + (delete o1.nonexistent));
console.log("op_computed=" + (delete o1["b"]) + ",empty=" + (Object.keys(o1).length === 0));
const o2: any = { inner: { deep: 5 } };
console.log("op_nested=" + (delete o2.inner.deep) + ",json=" + JSON.stringify(o2));
const dynKey = "foo";
const o3: any = { foo: 1 };
console.log("op_dynamic=" + (delete o3[dynKey]) + ",has=" + ("foo" in o3));
console.log("op_non_reference=" + (delete (1 + 1 as any)));
console.log("op_literal=" + (delete ("abc" as any)));
let sideEffects = 0;
function bump(): any { sideEffects++; return { a: 1 }; }
console.log("op_call=" + (delete bump().a) + ",calls=" + sideEffects);

// inherited key: delete looks only at own properties
const protoD: any = { shared: "P" };
const childD: any = Object.create(protoD);
childD.shared = "C";
console.log("shadow_before=" + childD.shared);
console.log("shadow_delete=" + Reflect.deleteProperty(childD, "shared") + ",after=" + childD.shared);
console.log("shadow_again=" + Reflect.deleteProperty(childD, "shared") + ",after=" + childD.shared);
console.log("proto_intact=" + protoD.shared);
console.log("inherited_only=" + Reflect.deleteProperty(childD, "toString") + ",still=" + typeof childD.toString);

// ---------- 2. Refusals: non-configurable, sealed, frozen, non-extensible ----------
const nc: any = { loose: 1 };
Object.defineProperty(nc, "fixed", { value: 2, configurable: false, enumerable: true, writable: true });
Object.defineProperty(nc, "fixedAcc", { get() { return 3; }, configurable: false, enumerable: true });
console.log("loose=" + Reflect.deleteProperty(nc, "loose") + ",left=" + Object.keys(nc).join("|"));
console.log("fixed=" + Reflect.deleteProperty(nc, "fixed") + ",read=" + nc.fixed);
console.log("fixedAcc=" + Reflect.deleteProperty(nc, "fixedAcc") + ",read=" + nc.fixedAcc);

const sealed: any = Object.seal({ a: 1, b: 2 });
console.log("sealed=" + Reflect.deleteProperty(sealed, "a") + ",keys=" + Object.keys(sealed).join("|"));
const frozen: any = Object.freeze({ a: 1 });
console.log("frozen=" + Reflect.deleteProperty(frozen, "a"));
const nonExt: any = Object.preventExtensions({ a: 1 });
console.log("nonext=" + Reflect.deleteProperty(nonExt, "a") + ",keys=" + Object.keys(nonExt).join("|"));
console.log("nonext_readd=" + Reflect.set(nonExt, "a", 1) + ",has=" + Object.hasOwn(nonExt, "a"));

// arrays: indices are configurable, length is not, a frozen array refuses element delete
const arr: any = [1, 2, 3];
console.log("arr_index=" + Reflect.deleteProperty(arr, "1") + ",len=" + arr.length + ",json=" + JSON.stringify(arr));
console.log("arr_hole=" + Object.hasOwn(arr, "1") + ",read=" + arr[1] + ",in=" + (1 in arr));
console.log("arr_string=" + String(arr));
console.log("arr_length=" + Reflect.deleteProperty(arr, "length") + ",len=" + arr.length);
console.log("arr_oob=" + Reflect.deleteProperty(arr, "99"));
const frozenArr: any = Object.freeze([1, 2]);
console.log("frozen_arr=" + Reflect.deleteProperty(frozenArr, "0") + ",v=" + frozenArr[0]);

// boxed string: index and length are non-configurable; extra props are fine
const boxed: any = Object(" ab");
console.log("str_index=" + Reflect.deleteProperty(boxed, "1") + ",read=" + boxed[1]);
console.log("str_length=" + Reflect.deleteProperty(boxed, "length"));
console.log("str_added=" + Reflect.set(boxed, "extra", 1) + ",del=" + Reflect.deleteProperty(boxed, "extra"));

// functions: name and length are configurable, prototype is not
function fnMeta(_a: number): void { /* noop */ }
console.log("fn_name=" + Reflect.deleteProperty(fnMeta, "name") + ",name=" + JSON.stringify(fnMeta.name));
console.log("fn_length=" + Reflect.deleteProperty(fnMeta, "length") + ",length=" + fnMeta.length);
console.log("fn_prototype=" + Reflect.deleteProperty(fnMeta, "prototype") + ",proto=" + typeof fnMeta.prototype);

// proxies: the trap is consulted, and a true answer for a non-configurable key breaks the invariant
const seen: string[] = [];
const tgt: any = { p: 1 };
Object.defineProperty(tgt, "nc", { value: 2, configurable: false });
const px: any = new Proxy(tgt, {
  deleteProperty(target, key) { seen.push(String(key)); return Reflect.deleteProperty(target, key); },
});
console.log("proxy_p=" + Reflect.deleteProperty(px, "p"));
console.log("proxy_nc=" + Reflect.deleteProperty(px, "nc"));
console.log("proxy_seen=" + seen.join("|"));
const liar: any = new Proxy(tgt, { deleteProperty() { return true; } });
try {
  console.log("proxy_liar=" + Reflect.deleteProperty(liar, "nc"));
} catch (e: any) {
  console.log("proxy_liar=throw:" + e.constructor.name);
}
console.log("nc_still=" + Object.hasOwn(tgt, "nc"));

// ---------- 3. Clone idioms: spread / assign flatten, descriptor copy preserves ----------
const proto: any = { fromProto: "P" };
const symKey = Symbol("sk");
let backing = "B0";
const src: any = Object.create(proto);
src.plain = 1;
Object.defineProperty(src, "acc", {
  get() { return "acc:" + backing; },
  set(v: string) { backing = v; },
  enumerable: true,
  configurable: true,
});
Object.defineProperty(src, "hidden", { value: "H", enumerable: false, writable: true, configurable: true });
Object.defineProperty(src, "locked", { value: "L", enumerable: true, writable: false, configurable: false });
src[symKey] = "S";

console.log("desc_data_keys=" + Object.keys(Object.getOwnPropertyDescriptor(src, "plain") as any).join("|"));
console.log("desc_acc_keys=" + Object.keys(Object.getOwnPropertyDescriptor(src, "acc") as any).join("|"));
const all = Object.getOwnPropertyDescriptors(src);
console.log("gopds_keys=" + Reflect.ownKeys(all).map(String).join("|"));
console.log("gopds_skips_proto=" + ("fromProto" in all));

const clone: any = Object.create(Object.getPrototypeOf(src), all);
console.log("clone_proto=" + (Object.getPrototypeOf(clone) === proto) + ",inherited=" + clone.fromProto);
console.log("clone_keys=" + Reflect.ownKeys(clone).map(String).join("|"));
console.log("clone_acc_kind=" + ("get" in (Object.getOwnPropertyDescriptor(clone, "acc") as any) ? "accessor" : "data"));
clone.acc = "B1";
console.log("clone_setter_shared=" + backing + ",src_reads=" + src.acc);
const cHidden = Object.getOwnPropertyDescriptor(clone, "hidden") as any;
console.log("clone_hidden=e=" + cHidden.enumerable + ",v=" + cHidden.value);
const cLocked = Object.getOwnPropertyDescriptor(clone, "locked") as any;
console.log("clone_locked=w=" + cLocked.writable + ",c=" + cLocked.configurable);
console.log("clone_sym=" + clone[symKey]);

const spread: any = { ...src };
console.log("spread_proto=" + (Object.getPrototypeOf(spread) === Object.prototype));
console.log("spread_keys=" + Reflect.ownKeys(spread).map(String).join("|"));
const sAcc = Object.getOwnPropertyDescriptor(spread, "acc") as any;
console.log("spread_acc=" + ("get" in sAcc ? "accessor" : "data") + ",v=" + sAcc.value);
const sLocked = Object.getOwnPropertyDescriptor(spread, "locked") as any;
console.log("spread_locked=w=" + sLocked.writable + ",c=" + sLocked.configurable);

const assigned: any = Object.assign({}, src);
console.log("assign_keys=" + Reflect.ownKeys(assigned).map(String).join("|"));

console.log("missing_desc=" + Object.getOwnPropertyDescriptor(src, "nope"));
console.log("inherited_desc=" + Object.getOwnPropertyDescriptor(src, "fromProto"));

// cloning an array with descriptors keeps the length as a data slot that tracks nothing
const arrSrc: any = [1, 2, 3];
const arrClone: any = Object.create(Object.getPrototypeOf(arrSrc), Object.getOwnPropertyDescriptors(arrSrc));
console.log("arrclone_isarray=" + Array.isArray(arrClone) + ",len=" + arrClone.length);
arrClone[3] = 4;
console.log("arrclone_len_after=" + arrClone.length + ",src=" + arrSrc.length);
console.log("arrclone_tag=" + Object.prototype.toString.call(arrClone));
const lenDesc = Object.getOwnPropertyDescriptor(arrSrc, "length") as any;
console.log("arr_len_desc=w=" + lenDesc.writable + ",e=" + lenDesc.enumerable + ",c=" + lenDesc.configurable);

// descriptors of a primitive are taken from its box
console.log("string_descs=" + Reflect.ownKeys(Object.getOwnPropertyDescriptors("ab" as any)).map(String).join("|"));
console.log("number_descs=" + Reflect.ownKeys(Object.getOwnPropertyDescriptors(5 as any)).length);

// a descriptor's value is a live reference, not a snapshot of the object graph
const live: any = { o: { n: 1 } };
const liveDescs: any = Object.getOwnPropertyDescriptors(live);
live.o.n = 2;
console.log("live_ref=" + liveDescs.o.value.n);

// Object.create rejects a descriptor that mixes value and accessor fields
try {
  Object.create(null, { bad: { value: 1, get() { return 2; } } as any });
  console.log("create_both=ok");
} catch (e: any) {
  console.log("create_both=throw:" + e.constructor.name);
}
