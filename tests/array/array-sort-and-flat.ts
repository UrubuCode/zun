// Sorting and flattening. Sort: the default string comparator, comparator
// return coercion (NaN and undefined mean "equal"), stability, where undefined
// and holes are placed, accessor side effects, and sort-versus-toSorted. Flat
// and flatMap: depth coercion, hole removal, array-like handling, and species.
// Comparators are kept consistent, because an inconsistent comparator produces an
// order that is implementation-defined and would differ between engines.

// Default sort compares ToString of each element by UTF-16 code units.
const nums = [10, 9, 1, 100, 2, 20];
console.log("defaultSort=" + nums.slice().sort().join(","));
console.log("numericSort=" + nums.slice().sort((a, b) => a - b).join(","));
console.log("lexNumbers=" + [10, 1, 2, 21, 3, 100, 13].slice().sort().join(","));
console.log("lexFive=" + [5, 50, 500, 9, 90].slice().sort().join(",") + " lexNeg=" + [-1, -10, -2, 0, 1].slice().sort().join(","));
console.log("lexFrac=" + [1.5, 1.25, 1.125, 11].slice().sort().join(","));
const objs: any[] = [{ toString: () => "b" }, { toString: () => "a" }];
console.log("byToString=" + objs.slice().sort().join(","));
console.log("caseOrder=" + ["b", "A", "a", "B", "1", "_"].sort().join("") + " unicode=" + [String.fromCharCode(0xe9), "z", "a"].sort().join(""));
console.log("numbersSortInPlace=" + [3, 2, 10, 1].sort().join(",") + " strings=" + ["banana", "apple", "cherry"].sort().join(","));
const sortIn = [3, 1, 4, 1, 5];
console.log("sortIdentity=" + (sortIn.sort() === sortIn) + " sorted=" + sortIn.join(","));
const src2 = [3, 1, 2];
const copy2 = src2.toSorted();
console.log("toSortedNew=" + (copy2 !== src2) + " src=" + src2.join(",") + " out=" + copy2.join(","));
console.log("toSortedDesc=" + [3, 1, 4].toSorted((a, b) => b - a).join(","));
console.log("sortInPlaceNumeric=" + [10, 5, 40, 25, 1000, 1].sort((a, b) => a - b).join(","));

// Comparator return values: NaN and undefined compare as equal, so the stable
// input order is kept. A negative or positive number orders the pair.
const cmpIn = [4, 2, 6, 1, 3, 5];
console.log("nanCmp=" + cmpIn.slice().sort(() => NaN).join(","));
console.log("undefCmp=" + ["b", "a", "c"].slice().sort(() => undefined as any).join(","));
console.log("negZeroCmp=" + [3, 1, 2].slice().sort(() => -0).join(","));
console.log("strCmpNumeric=" + [3, 1, 2].slice().sort((a, b) => (a < b ? -1 : a > b ? 1 : 0)).join(","));

// Stability is required (ES2019): equal keys keep their input order.
const recs = [
  { k: 1, id: "a" }, { k: 0, id: "b" }, { k: 1, id: "c" },
  { k: 0, id: "d" }, { k: 1, id: "e" }, { k: 0, id: "f" },
  { k: 1, id: "g" }, { k: 0, id: "h" }, { k: 1, id: "i" },
  { k: 0, id: "j" }, { k: 1, id: "k" }, { k: 0, id: "l" },
];
console.log("stable=" + recs.slice().sort((x, y) => x.k - y.k).map((r) => r.id).join(""));
const tie = [{ k: 2, id: "a" }, { k: 1, id: "b" }, { k: 2, id: "c" }, { k: 1, id: "d" }];
tie.sort((x, y) => x.k - y.k);
console.log("tieStable=" + tie.map((x) => x.k + x.id).join(","));

// The comparator is not called for arrays of fewer than two elements.
let calls = 0;
[1].sort(() => { calls++; return 0; });
([] as number[]).sort(() => { calls++; return 0; });
console.log("smallCalls=" + calls);

// A comparator that is not a function (and not undefined) is a TypeError.
try { ([2, 1] as any).sort(null); } catch (e: any) { console.log("nullCmp=" + e.name); }
try { ([2, 1] as any).sort(5); } catch (e: any) { console.log("numCmp=" + e.name); }
console.log("explicitUndefCmp=" + [2, 1].sort(undefined).join(","));

// Where undefined and holes go: undefined sorts after every value, holes after
// undefined, and neither is ever passed to the comparator.
const su: any[] = [3, undefined, 1, undefined, 2];
console.log("undefPlacement=" + JSON.stringify(su.sort()));
const cmpSeen: boolean[] = [];
const sb: any[] = [3, undefined, 1];
sb.sort((x, y) => { cmpSeen.push(x === undefined || y === undefined); return x - y; });
console.log("comparatorSawUndef=" + cmpSeen.includes(true) + " result=" + JSON.stringify(sb));
const holes: any[] = [5, , 1, , 3];
holes.sort();
console.log("holesSorted=" + holes.length + " in=" + [0, 1, 2, 3, 4].map((i) => (i in holes ? "y" : "n")).join("") + " vals=" + holes.map(String).join(","));
const mixedHoles: any[] = [2, , undefined, 1];
mixedHoles.sort();
console.log("mixedHoles=" + mixedHoles.length + " in=" + [0, 1, 2, 3].map((i) => (i in mixedHoles ? "y" : "n")).join("") + " m0=" + String(mixedHoles[0]) + " m2=" + String(mixedHoles[2]));
const ts = [5, , 1, undefined, 3].toSorted();
console.log("toSortedHoles=" + ts.length + " in=" + [0, 1, 2, 3, 4].map((i) => (i in ts ? "y" : "n")).join("") + " v=" + ts.map(String).join(","));

// Accessors: sort reads each element once (through the getter) and writes the
// sorted values back through the setter.
const acc: any[] = [3, 2, 1];
const accSeen: string[] = [];
Object.defineProperty(acc, "0", {
  configurable: true, enumerable: true,
  get() { accSeen.push("get0"); return 3; },
  set(v) { accSeen.push("set0:" + v); },
});
acc.sort((x, y) => x - y);
console.log("accessorSort=" + acc[1] + "," + acc[2] + " events=" + accSeen.join("|"));

// Flat: depth is coerced with ToIntegerOrInfinity; 0 copies, negative and NaN do nothing.
const deep = [1, [2, [3, [4, [5]]]]];
console.log("flatDepth0=" + JSON.stringify(deep.flat(0)) + " flatDepth1=" + JSON.stringify(deep.flat(1)));
console.log("flatDepth2=" + JSON.stringify(deep.flat(2)) + " flatDepthInf=" + JSON.stringify(deep.flat(Infinity)));
console.log("flatDefault=" + JSON.stringify([1, 2, 3].flat()) + " flatNeg=" + JSON.stringify(deep.flat(-1)) + " flatNaN=" + JSON.stringify(deep.flat(NaN)));
console.log("flatStrDepth=" + JSON.stringify(deep.flat("2" as any)) + " flatFracDepth=" + JSON.stringify(deep.flat(1.9)));

// Flat removes holes at every level, and flattens only real arrays: array-likes and strings stay whole.
const h: any[] = [1, , 2, [3, , 4], , 5];
console.log("flatHoles=" + JSON.stringify(h.flat()) + " len=" + h.flat().length);
const al: any[] = [[1], { length: 2, 0: "a", 1: "b" }, "xy"];
console.log("flatArrayLike=" + JSON.stringify(al.flat()));
console.log("flatMapBasic=" + JSON.stringify([1, 2, 3].flatMap((x) => [x, x * 10])) + " flatMapNested=" + JSON.stringify([1, 2].flatMap((v) => [[v]])));
console.log("flatMapEmpty=" + [1, 2, 3].flatMap((n) => (n % 2 ? [] : [n])).join(",") + " flatMapScalar=" + [1, 2, 3].flatMap((n) => n + 1).join(","));
console.log("flatMapPairs=" + [1, 2].flatMap((n) => [[n, n + 1]]).map((x) => JSON.stringify(x)).join("|"));
console.log("flatMapHoles=" + JSON.stringify([1, , 2].flatMap((v) => [v, v])) + " flatMapOne=" + JSON.stringify([1, 2, 3].flatMap((x) => [x, [x]])));
console.log("flatMapOfFlat=" + JSON.stringify([[1], [2], [3]].flatMap((x) => [x[0], x[0] * 10])));
try { ([1, , 2] as any).flatMap(1); } catch (e: any) { console.log("flatMapBadCb=" + e.name); }

// flat and flatMap build their result with species.
class MyArr extends Array {}
const my: any = MyArr.from([1, [2]] as any);
console.log("flatSpecies=" + (my.flat() instanceof MyArr) + " flatMapSpecies=" + (my.flatMap((x: any) => [x]) instanceof MyArr));

// Sorting and mutation together: a fixed pseudo-random program of array
// operations, replayed against the same LCG. Each step prints a checksum, so the
// first differing line names the operation that diverged.
let seed = 12345;
function rnd(n: number): number {
  seed = (seed * 1103515245 + 12345) % 2147483648;
  return seed % n;
}
function checksum(xs: any[]): string {
  let hsh = 0;
  for (let i = 0; i < xs.length; i++) {
    const present = Object.prototype.hasOwnProperty.call(xs, i);
    const v = present ? String(xs[i]) : " hole";
    for (let j = 0; j < v.length; j++) hsh = (hsh * 31 + v.charCodeAt(j) + i) % 1000003;
  }
  return xs.length + ":" + hsh;
}
const ops = [
  "push", "pop", "shift", "unshift", "splice-del", "splice-ins", "reverse",
  "sort", "fill", "copyWithin", "hole", "setLength", "delete", "at-write",
];
const prog: any[] = [5, 3, 9, 1, 7];
console.log("torture start=" + checksum(prog));
for (let step = 0; step < 60; step++) {
  const op = ops[rnd(ops.length)];
  const i = prog.length ? rnd(prog.length) : 0;
  const v = rnd(20);
  switch (op) {
    case "push": prog.push(v); break;
    case "pop": prog.pop(); break;
    case "shift": prog.shift(); break;
    case "unshift": prog.unshift(v); break;
    case "splice-del": prog.splice(i, rnd(3)); break;
    case "splice-ins": prog.splice(i, 0, v, v + 1); break;
    case "reverse": prog.reverse(); break;
    case "sort": prog.sort((x: any, y: any) => (x === undefined ? 0 : x) - (y === undefined ? 0 : y)); break;
    case "fill": prog.fill(v, i, i + 2); break;
    case "copyWithin": prog.copyWithin(i, rnd(prog.length + 1)); break;
    case "hole": prog[prog.length + rnd(3)] = v; break;
    case "setLength": prog.length = Math.max(0, prog.length - rnd(2)); break;
    case "delete": delete prog[i]; break;
    case "at-write": if (prog.length) prog[prog.length - 1] = v; break;
  }
  console.log("torture step=" + step + " op=" + op + " sum=" + checksum(prog));
}
console.log("finalLen=" + prog.length + " keys=" + Object.keys(prog).join(","));
console.log("finalJoin=" + prog.join("|") + " json=" + JSON.stringify(prog));
console.log("finalSpread=" + [...prog].map((x) => String(x)).join(",") + " visits=" + (() => { let n = 0; prog.forEach(() => n++); return n; })());
console.log("finalSorted=" + prog.slice().sort().join(","));
console.log("finalReduced=" + prog.reduce((x: any, y: any) => (x === undefined ? 0 : x) + (y === undefined ? 0 : y), 0));
