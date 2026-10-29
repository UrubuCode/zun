// The length property and index keys: truncation and growth through assignment
// and defineProperty, non-configurable elements that block truncation, the
// non-writable length attribute, which property keys count as array indices,
// and the 2^32-2 boundary. Reflect is used where a boolean outcome matters, so
// the result does not depend on strict or sloppy mode.

function len(a: any[]) { return a.length + "[" + Object.keys(a).join(",") + "]"; }

// Length as a plain data property: truncation deletes, growth leaves holes.
const t = [0, 1, 2, 3, 4];
t.length = 2;
console.log("trunc=" + JSON.stringify(t) + " len=" + t.length + " has2=" + (2 in t));
t.length = 5;
console.log("grow=" + t.length + " holes=" + [0, 1, 2, 3, 4].map((i) => (i in t ? "y" : "n")).join(""));
t.length = 0;
console.log("cleared=" + t.length + " join=[" + t.join(",") + "]");

// A string length is coerced, a bad length throws RangeError rather than coercing.
const sl = [1, 2, 3];
sl.length = "1" as any;
console.log("strLen=" + sl.length + " " + JSON.stringify(sl) + " type=" + typeof sl.length);
for (const bad of [-1, 1.5, NaN, 4294967296, Infinity]) {
  const c = [1, 2, 3];
  try { c.length = bad as any; console.log("badLengthAccepted(" + String(bad) + ")=" + c.length); }
  catch (e: any) { console.log("badLength(" + String(bad) + ")=" + e.name); }
}

// The length descriptor: writable, not enumerable, not configurable.
const ld: any = Object.getOwnPropertyDescriptor([1], "length");
console.log("lengthDesc=" + [ld.writable, ld.enumerable, ld.configurable].join(","));

// A non-configurable element stops truncation at its own index; length reports
// how far it got and the set reports failure.
const blocked = [0, 1, 2, 3, 4];
Object.defineProperty(blocked, 2, { value: 22, configurable: false });
console.log("blockedSet=" + Reflect.set(blocked, "length", 0) + " len=" + blocked.length + " kept=" + blocked.map(String).join(","));
console.log("blockedAgain=" + Reflect.set(blocked, "length", 1) + " " + len(blocked));
console.log("blockedGrow=" + Reflect.set(blocked, "length", 9) + " len=" + blocked.length);

// defineProperty on length: a smaller value truncates, and the attribute is
// applied after the truncation, so making length read-only is one-way.
const dp = [0, 1, 2, 3];
console.log("defineSmaller=" + Reflect.defineProperty(dp, "length", { value: 2 }) + " " + len(dp));
const lock = [0, 1, 2, 3];
console.log("shrinkAndLock=" + Reflect.defineProperty(lock, "length", { value: 1, writable: false }) + " " + len(lock));
console.log("lockedDesc=" + JSON.stringify(Object.getOwnPropertyDescriptor(lock, "length")));
console.log("pushAfterLock=" + (() => { try { lock.push(9); return "no-throw:" + lock.length; } catch (e: any) { return e.name + ":" + lock.length; } })());
console.log("setAfterLock=" + Reflect.set(lock, "length", 5) + " " + len(lock));
console.log("indexWriteAfterLock=" + Reflect.set(lock, 1, "x") + " " + len(lock));
console.log("index0WriteAfterLock=" + Reflect.set(lock, 0, "y") + " v=" + lock[0]);
console.log("unlock=" + Reflect.defineProperty(lock, "length", { writable: true }));
console.log("sameValueRedefine=" + Reflect.defineProperty(lock, "length", { value: 1, writable: false }));
const attrs = [1];
console.log("makeEnumerable=" + Reflect.defineProperty(attrs, "length", { enumerable: true }));
console.log("makeConfigurable=" + Reflect.defineProperty(attrs, "length", { configurable: true }));
console.log("makeAccessor=" + Reflect.defineProperty(attrs, "length", { get() { return 0; } }));
console.log("attrsDesc=" + JSON.stringify(Object.getOwnPropertyDescriptor(attrs, "length")));

// Freezing makes length non-writable, so the write is refused outright.
const frozen = Object.freeze([1, 2]);
console.log("frozenSet=" + Reflect.set(frozen, "length", 0) + " len=" + frozen.length + " writable=" + Object.getOwnPropertyDescriptor(frozen, "length")!.writable);

// Defining an index beyond length grows it; defining one below does not shrink it.
const grow = [0];
console.log("defineIndex5=" + Reflect.defineProperty(grow, 5, { value: "five", enumerable: true, writable: true, configurable: true }) + " " + len(grow));
console.log("defineIndex0=" + Reflect.defineProperty(grow, 0, { value: "zero" }) + " " + len(grow));
console.log("defineStrKey=" + Reflect.defineProperty(grow, "007", { value: "bond", enumerable: true }) + " " + len(grow));
console.log("defineFloatKey=" + Reflect.defineProperty(grow, "1.5", { value: "half", enumerable: true }) + " " + len(grow));

// Accessor-defined indices count for length, iteration and JSON.
const acc: any[] = [];
Reflect.defineProperty(acc, 0, { get() { return "computed"; }, enumerable: true, configurable: true });
console.log("accessorIndex=" + len(acc) + " v=" + acc[0] + " join=" + acc.join(",") + " json=" + JSON.stringify(acc));
console.log("accessorMap=" + JSON.stringify(acc.map((x: any) => x)));

// A non-enumerable index is skipped by Object.keys and for-in, but not by the
// array methods or by JSON.
const nonEnum: any[] = [1, 2];
Reflect.defineProperty(nonEnum, 1, { enumerable: false });
console.log("nonEnumIndex=" + len(nonEnum) + " join=" + nonEnum.join(",") + " json=" + JSON.stringify(nonEnum));
console.log("nonEnumForEach=" + (() => { let n = 0; nonEnum.forEach(() => n++); return n; })());
console.log("nonEnumForIn=" + (() => { const o: string[] = []; for (const k in nonEnum) o.push(k); return o.join(","); })());

// A shrink blocked by a non-configurable index keeps the elements above it
// removed and reports failure; Reflect.set makes the outcome mode-independent.
const rb: any[] = [0, 1, 2, 3, 4];
Object.defineProperty(rb, "2", { configurable: false, writable: true, value: 2 });
console.log("rollback=" + Reflect.set(rb, "length", 1) + " len=" + rb.length + " keys=" + Object.keys(rb).join(","));
console.log("rollbackValues=" + rb[2] + "," + rb[3] + "," + rb[4]);

// The same defineProperty-driven truncation, with length set by defineProperty
// on an array that has an explicit index beyond the old end.
const dq: any[] = [1, 2, 3, 4];
Object.defineProperty(dq, "6", { value: 7, enumerable: true, configurable: true, writable: true });
console.log("afterDefine=" + dq.length + ":" + dq.join("|") + ":" + Object.keys(dq).join(","));
dq.length = 2;
console.log("afterTrunc=" + dq.length + ":" + dq.join("|") + ":" + Object.keys(dq).join(","));
try { Object.defineProperty(dq, "length", { value: 1.5 }); } catch (e: any) { console.log("defineBadLength=" + e.name); }
console.log("finalDq=" + dq.length + ":" + dq.join("|"));

// Index-key canonicalisation: only the canonical decimal form of an integer in
// [0, 2^32-2) is an array index. Everything else is an ordinary string key.
const k: any = [];
k[0] = "zero";
k["1"] = "one";
k["01"] = "not-index";
k["1.0"] = "not-index";
k[" 2"] = "not-index";
k["-0"] = "not-index";
k["1e2"] = "not-index";
k["+3"] = "not-index";
console.log("keyLen=" + k.length + " keys=" + Object.keys(k).join("|"));
console.log("keyJson=" + JSON.stringify(k));
const neg: any = [1];
neg[-1] = "neg";
neg[1.5] = "frac";
console.log("negKeyLen=" + neg.length + " keys=" + Object.keys(neg).join("|") + " at=" + neg.at(-1) + " idx=" + neg[-1]);
const same: any = [];
same[2] = "x";
console.log("sameKey=" + (same["2"] === same[2]) + " has=" + ("2" in same) + " len=" + same.length);
const sparseKey: any = [];
sparseKey[5] = "five";
console.log("sparseKey=" + sparseKey.length + " in0=" + (0 in sparseKey) + " keys=" + Object.keys(sparseKey).join("|") + " join=" + sparseKey.join(","));
const order: any = [];
order[3] = "d"; order[1] = "b"; order.zz = "z"; order[0] = "a"; order.aa = "y";
const forInOrder: string[] = [];
for (const key in order) forInOrder.push(key);
console.log("forInOrder=" + forInOrder.join(","));
console.log("ownKeys=" + Reflect.ownKeys(order).map(String).join(","));
const sym = Symbol("s");
const withSym: any = [1];
withSym[sym] = "sym";
console.log("symLen=" + withSym.length + " symKeys=" + Object.keys(withSym).join("|") + " symJson=" + JSON.stringify(withSym));

// The 2^32-2 boundary: the largest index stores sparsely and sets length to
// 2^32-1; 2^32-1 itself is an ordinary string key and does not touch length.
const maxIdx: any = [];
maxIdx[4294967294] = "last";
console.log("maxIndexLen=" + maxIdx.length + " val=" + maxIdx[4294967294] + " keys=" + Object.keys(maxIdx).join("|"));
console.log("maxIndexIn=" + (0 in maxIdx) + " " + (4294967294 in maxIdx));
const beyond: any = [];
beyond[4294967295] = "beyond";
console.log("beyondLen=" + beyond.length + " keys=" + Object.keys(beyond).join("|") + " val=" + beyond[4294967295]);
const above: any = [];
above[4294967296] = "way-beyond";
console.log("aboveLen=" + above.length + " keys=" + Object.keys(above).join("|"));
const maxLen: any = [];
maxLen.length = 4294967295;
console.log("setMaxLen=" + maxLen.length + " keys=" + Object.keys(maxLen).length);
try { maxLen.length = 4294967296; } catch (e: any) { console.log("setBeyond=" + e.name); }
console.log("stillMax=" + maxLen.length);
console.log("ctorMax=" + new Array(4294967295).length);
try { new Array(4294967296); } catch (e: any) { console.log("ctorBeyond=" + e.name); }
try { new Array(-1); } catch (e: any) { console.log("ctorNeg=" + e.name); }
try { new Array(1.5); } catch (e: any) { console.log("ctorFrac=" + e.name); }
// Only O(1) queries here on purpose: a scan over a 4-billion-slot array would
// measure the walk rather than the representation.
const huge: any = [];
huge[4000000000] = "x";
console.log("hugeLen=" + huge.length + " hugeKeys=" + Object.keys(huge).length + " hugeLast=" + huge.at(-1));
console.log("hugeHas=" + (4000000000 in huge) + " " + (3999999999 in huge));
