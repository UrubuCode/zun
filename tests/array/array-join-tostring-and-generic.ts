// Turning arrays into strings and primitives, and applying Array.prototype
// methods to things that are not arrays: join (separators, nullish and nested
// values, cycle detection), toString (delegation to join), toLocaleString,
// ToPrimitive hints, loose and strict comparison, and the generic methods
// applied to array-likes, strings, and non-objects.

// join: default and explicit separators; null and undefined become empty strings.
const j = [1, 2, 3];
console.log("joinDefault=" + j.join() + " comma=" + j.join(",") + " dash=" + j.join("-") + " empty=" + j.join("") + " space=" + j.join(" "));
console.log("joinUndef=" + [1, undefined, 3].join(",") + " joinNull=" + [1, null, 3].join(",") + " joinEmptyStr=" + [1, "", 3].join(","));
console.log("joinEmptyArr=[" + ([] as number[]).join(",") + "] joinSingle=" + [5].join("-") + " joinNested=" + [1, [2, [3, [4]]]].join(","));
console.log("joinObjects=" + [{ toString: () => "T" }, { toString: () => "U" }].join("+"));
console.log("joinSeparatorObj=" + [1, 2].join({ toString: () => "/" } as any));
console.log("joinSeparatorUndef=" + [1, 2].join(undefined) + " joinSeparatorNull=" + [1, 2].join(null as any));

// Cycle detection: an array reachable from itself contributes an empty string.
const self: any[] = [1, 2];
self.push(self);
console.log("selfJoin=" + self.join(",") + " selfString=" + String(self) + " again=" + self.join(","));
const mb: any[] = [1];
const mc: any[] = [2, mb];
mb.push(mc);
console.log("mutualB=" + mb.join("-") + " mutualC=" + mc.join("-"));
const dd: any[] = [];
dd.push(dd, dd, 9);
console.log("tripleSelf=" + dd.join("|"));
const plainObj: any = { toString() { return "O"; } };
console.log("objectNotGuarded=" + [plainObj, 1].join(","));

// toString delegates to join when join is callable, and falls back otherwise.
console.log("toStringPlain=" + [1, 2, 3].toString());
const own: any = [1, 2, 3];
own.join = function () { return "OWN"; };
console.log("ownJoin=" + own.toString() + " string=" + String(own) + " template=" + `${own}`);
const notFn: any = [1, 2, 3];
notFn.join = 42;
console.log("nonCallableJoin=" + notFn.toString());
const fake: any = { join() { return "FAKE"; }, length: 0 };
console.log("fakeJoin=" + Array.prototype.toString.call(fake) + " noJoin=" + Array.prototype.toString.call({ length: 0 } as any));
class L extends Array { join() { return "SUB"; } }
const sub = new L();
sub.push(7);
console.log("subclassToString=" + sub.toString() + " len=" + sub.length);
console.log("toStringNested=" + String([1, [2, [3]], null, undefined, [null]]) + " emptyStr=" + JSON.stringify(String([])) + " singleNull=" + JSON.stringify(String([null])));
console.log("toStringHoles=" + JSON.stringify(String([, , ])) + " mixedToString=" + [1, "hello", true].toString());

// toLocaleString delegates to each element's toLocaleString; for numbers only the
// shape is probed, not the locale formatting.
const t: any = [{ toLocaleString: () => "L1" }, { toLocaleString: () => "L2" }];
console.log("toLocaleCustom=" + t.toLocaleString() + " toLocaleNulls=" + JSON.stringify([null, undefined, "x"].toLocaleString()));
console.log("toLocaleSmall=" + [1, 2, 3].toLocaleString() + " toLocaleEmpty=" + JSON.stringify(([] as number[]).toLocaleString()));
console.log("toLocaleMixed=" + [1, "hello", true].toLocaleString().includes("hello") + " toLocaleStrTest=" + ["a", "b"].toLocaleString());

// ToPrimitive on an array: no Symbol.toPrimitive, so valueOf is tried first (it answers
// an object, which is rejected), then toString. A primitive from valueOf wins.
const calls: string[] = [];
const a: any = [1, 2];
a.valueOf = function () { calls.push("valueOf"); return this; };
a.toString = function () { calls.push("toString"); return "STR"; };
console.log("plus=" + (a + "") + " order=" + calls.join(","));
calls.length = 0;
console.log("template=" + `${a}` + " order=" + calls.join(","));
calls.length = 0;
console.log("numeric=" + Number(a) + " order=" + calls.join(","));
const b: any = [1, 2];
const bCalls: string[] = [];
b.valueOf = function () { bCalls.push("valueOf"); return 42; };
b.toString = function () { bCalls.push("toString"); return "NO"; };
console.log("primValueOf=" + (b + 1) + " order=" + bCalls.join(","));
bCalls.length = 0;
console.log("primString=" + String(b) + " order=" + bCalls.join(","));
const c: any = [1];
const cCalls: string[] = [];
c.valueOf = function () { cCalls.push("valueOf"); return 7; };
c.toString = function () { cCalls.push("toString"); return "C"; };
console.log("stringHint=" + String(c) + " order=" + cCalls.join(","));
cCalls.length = 0;
console.log("bracketKey=" + ({ C: "hit" } as any)[c] + " order=" + cCalls.join(","));
const d: any = [1];
d[Symbol.toPrimitive] = function (hint: string) { return "hint:" + hint; };
console.log("spDefault=" + (d + "") + " spString=" + String(d) + " spNumber=" + Number(d));

// Relational comparison converts both sides with the number hint.
console.log("cmpArrays=" + ([2] < [11]) + " cmpNumbers=" + (2 < 11) + " cmpStrings=" + ([2] < ["11"]) + " " + ("2" < "11"));
// == coerces an array to a primitive; === never does.
console.log("looseEmpty=" + (([] as any) == false) + " strictEmpty=" + (([] as any) === false));
console.log("looseOne=" + (([1] as any) == 1) + " looseTwo=" + (([1, 2] as any) == "1,2"));
console.log("looseNull=" + (([] as any) == null) + " looseUndef=" + (([] as any) == undefined));

// Generic methods applied to an array-like object: they read and write by index
// and update length.
const like: any = { length: 3, 0: "a", 1: "b", 2: "c" };
console.log("genJoin=" + Array.prototype.join.call(like, "-") + " genSlice=" + JSON.stringify(Array.prototype.slice.call(like, 1)));
console.log("genIndexOf=" + Array.prototype.indexOf.call(like, "b") + " genIncludes=" + Array.prototype.includes.call(like, "c") + " genAt=" + Array.prototype.at.call(like, -1));
console.log("genMap=" + JSON.stringify(Array.prototype.map.call(like, (v: string) => v + "!")) + " genFilter=" + JSON.stringify(Array.prototype.filter.call(like, (v: string) => v !== "b")));
const stack: any = { length: 0 };
console.log("genPush=" + Array.prototype.push.call(stack, "x", "y") + " len=" + stack.length + " v=" + stack[0] + stack[1]);
console.log("genPop=" + Array.prototype.pop.call(stack) + " len=" + stack.length + " has1=" + (1 in stack));
console.log("genStrLength=" + Array.prototype.join.call({ length: "2", 0: 1, 1: 2, 2: 3 } as any, ","));
console.log("genNegLength=" + Array.prototype.join.call({ length: -5, 0: 1 } as any, ",") + "|");
console.log("genFracLength=" + Array.prototype.join.call({ length: 2.7, 0: 1, 1: 2, 2: 3 } as any, ","));
const sparseLike: any = { length: 3, 1: "mid" };
let visits = 0;
Array.prototype.forEach.call(sparseLike, () => visits++);
console.log("genSparseVisits=" + visits + " genSparseJoin=" + Array.prototype.join.call(sparseLike, ","));
console.log("genStringJoin=" + Array.prototype.join.call("abc", ".") + " genStringMap=" + JSON.stringify(Array.prototype.map.call("ab", (ch: string) => ch.toUpperCase())));
console.log("genStringSlice=" + JSON.stringify(Array.prototype.slice.call("abc", 1)));
console.log("genNumberSlice=" + JSON.stringify(Array.prototype.slice.call(5 as any)) + " genBoolJoin=" + Array.prototype.join.call(true as any, ",") + "|");
try { Array.prototype.join.call(null as any, ","); } catch (e: any) { console.log("genNull=" + e.name); }
try { Array.prototype.map.call(undefined as any, (x: any) => x); } catch (e: any) { console.log("genUndef=" + e.name); }
const genSlice: any = { length: 3, 0: "a", 2: "c" };
console.log("genSliceHoles=" + JSON.stringify(Array.prototype.slice.call(genSlice, 0)) + " genSliceIsArr=" + Array.isArray(Array.prototype.slice.call(genSlice, 0)));

// String tables as obfuscators emit them: index rotation by modular arithmetic,
// base64 decoding with atob, self-rotating tables, and member access through
// computed string keys.
const tbl = ["hello", "world", "foo", "bar", "baz", "result", "error", "ok", "done", "start"];
const pick = (idx: number, shift: number) => tbl[(idx + shift) % tbl.length];
console.log("tblPick=" + pick(0, 0) + "," + pick(1, 0) + "," + pick(0, 1) + "," + pick(9, 1));
console.log("b64Decoded=" + ["aGVsbG8=", "d29ybGQ=", "Zm9v", "YmFy"].map((s) => atob(s)).join(","));
const keyNames = ["l" + "e" + "n" + "g" + "t" + "h", "p" + "u" + "s" + "h", "j" + "o" + "i" + "n"];
const computed: any[] = [];
computed[keyNames[1]](1);
computed[keyNames[1]](2);
computed[keyNames[1]](3);
console.log("computedLength=" + computed[keyNames[0]] + " computedJoin=" + computed[keyNames[2]](","));
const rot = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"];
for (let i = 0; i < 2; i++) rot.push(rot.shift()!);
console.log("rotated=" + rot.join(",") + " at0=" + rot[0] + " at6=" + rot[6] + " at7=" + rot[7]);
console.log("rotatedMagic=" + rot[3] + "," + rot[4] + "," + rot[5]);
const enc = ["10", "20", "30", "40", "50", "60", "70", "80"];
const rotBy = enc.length - 3;
for (let i = 0; i < rotBy; i++) enc.push(enc.shift()!);
const dec = (idx: number) => parseInt(enc[idx], 10);
console.log("encRotated=" + enc.join(",") + " dec0=" + dec(0) + " dec3=" + dec(3) + " dec7=" + dec(7));
