// Holes: a missing index is not the same as an undefined element. This file
// tracks which methods skip holes, which read them as undefined, which keep
// them, how holes are enumerated, serialised, cloned, and inherited, and how
// callbacks observe holes that are filled or deleted while iterating.

const holeMask = (a: any[], n: number) => Array.from({ length: n }, (_, i) => (i in a ? "y" : "n")).join("");

// Basic: length counts holes; `in` and Object.keys do not see them.
const a = [1, , 3, , 5];
console.log("len=" + a.length + " has1=" + (1 in a) + " has3=" + (3 in a) + " keys=" + Object.keys(a).join(","));
console.log("sparseBasic=" + holeMask(a, 5) + " join=" + a.join("|") + " json=" + JSON.stringify(a));

// Skipping methods: forEach, map, filter and reduce never call back for a hole.
let count = 0;
a.forEach(() => { count++; });
console.log("forEachVisits=" + count + " mapLen=" + a.map((x) => x * 2).length + " mapIn1=" + (1 in a.map((x) => x * 2)));
console.log("mapValues=" + a.map((x) => (x === undefined ? "u" : x * 2)).join(",") + " filterAll=" + a.filter(() => true).join(","));
console.log("filterHoles=" + holeMask(a.filter(() => true), 3) + " mapSkips=" + holeMask(a.map((x) => x), 5));
console.log("reduceVisits=" + (() => { let n = 0; a.reduce((acc, x) => { n++; return acc; }, 0); return n; })());
console.log("fromReadsUndef=" + Array.from(a, (x) => (x === undefined ? "u" : x)).join(","));

// Reading methods: find, findIndex, includes and the iterator all see undefined.
console.log("findHole=" + String(a.find((x) => x === undefined)) + " findIdxHole=" + a.findIndex((x) => x === undefined));
console.log("includesHole=" + a.includes(undefined) + " indexOfHole=" + a.indexOf(undefined));
console.log("valuesHole=" + Array.from(a.values()).map(String).join(","));

// join and toString turn a hole into an empty string, like undefined and null.
const jh = [1, , 3];
console.log("joinHole=" + jh.join("-") + " joinUndef=" + [1, undefined, 3].join("-") + " joinNull=" + [1, null, 3].join("-"));
console.log("joinEmptyStr=" + [1, "", 3].join(",") + " joinEmptyArr=" + ([] as number[]).join(",") + " joinEmptyArg=" + [1, 2, 3].join(""));
console.log("joinSparseArr=" + Array(3).join(",") + " lenSparse=" + Array(3).length + " in0=" + (0 in Array(3)));
console.log("joinComma=" + [, , , ].length + " nullUndef=" + String([null, undefined]));
console.log("toStringHole=" + String([1, , 3]) + " mapHoleJoin=" + [, 2].map((x) => x).join(","));

// Serialisation routes: JSON turns holes into null, spread into undefined, and
// Object.keys never sees them. Each route gives a different answer.
const n: any[] = [1, , 3, , ];
console.log("nLen=" + n.length + " in=" + holeMask(n, 4) + " json=" + JSON.stringify(n));
console.log("jsonRoundTrip=" + JSON.parse(JSON.stringify(n)).map(String).join(",") + " rtIn=" + holeMask(JSON.parse(JSON.stringify(n)), 4));
const spread = [...n];
console.log("spreadIn=" + holeMask(spread, 4) + " v=" + spread.map(String).join(","));
console.log("keysOf=" + Object.keys(n).join(",") + " entries=" + Object.entries(n).map((p) => p[0] + ":" + String(p[1])).join(" "));
console.log("ownKeys=" + Reflect.ownKeys(n).map(String).join(","));
const forIn: string[] = [];
for (const k in n) forIn.push(k);
console.log("forIn=" + forIn.join(","));
const forOf: string[] = [];
for (const v of n) forOf.push(String(v));
console.log("forOf=" + forOf.join(","));
console.log("arrayFrom=" + Array.from(n).map(String).join(","));
const sc: any[] = structuredClone(n);
console.log("cloneIn=" + holeMask(sc, 4) + " len=" + sc.length);
console.log("nestedJson=" + JSON.stringify([[1, , 3], { k: undefined }, [, ]]));
console.log("objUndefJson=" + JSON.stringify({ a: undefined, b: 1 }) + " objNullJson=" + JSON.stringify({ a: null, b: 1 }));
const seenRep: string[] = [];
JSON.stringify([1, , 3], function (k, v) { seenRep.push(k + "=" + String(v)); return v; });
console.log("replacerSeesHole=" + seenRep.join(" "));
console.log("fnSymbolInArray=" + JSON.stringify([1, function () {}, Symbol("s"), undefined, 2]));
console.log("fnSymbolInObject=" + JSON.stringify({ a: 1, f() {}, s: Symbol("s"), u: undefined }));
console.log("literalCommas=" + [1, 2, ].length + " doubleComma=" + [1, , 2].length + " leadingHole=" + [, 1].length);

// Spread into an object literal copies own enumerable keys, so holes vanish there.
console.log("objSpreadHoles=" + JSON.stringify({ ...[1, , 3] }) + " objSpreadStr=" + JSON.stringify({ ..."ab" }));
console.log("objSpreadNull=" + JSON.stringify({ ...(null as any), k: 1 }));

// Sparse arrays from the constructor and from element assignment far past the end.
const ctorSparse = new Array(3);
console.log("ctorSparseIn=" + holeMask(ctorSparse, 3) + " forEachCount=" + (() => { let k = 0; ctorSparse.forEach(() => k++); return k; })());
const sp: any[] = [];
sp[5] = "five";
console.log("farAssign=" + sp.length + " in0=" + (0 in sp) + " keys=" + Object.keys(sp).join("|") + " join=" + sp.join(","));

// Spread and Array.from materialise holes as undefined; slice and concat keep them.
const hs: any[] = Array(3);
hs[1] = "x";
const hsSpread = [...hs];
const hsFrom = Array.from(hs);
console.log("spreadMaterial=" + hsSpread.map((v, i) => i + ":" + String(v)).join(",") + " keys=" + Object.keys(hsSpread).join(","));
console.log("fromMaterial=" + hsFrom.map((v, i) => i + ":" + String(v)).join(",") + " srcKeys=" + Object.keys(hs).join(","));

// An inherited numeric property fills a hole for reads and for the iteration
// methods, but Object.keys and JSON only see own properties.
const proto: any = { 1: "inherited" };
const inh: any = Object.create(proto);
inh[0] = "own";
inh[2] = "last";
inh.length = 3;
const inhSeen: string[] = [];
const inhOut = Array.prototype.filter.call(inh, (v: any, i: number) => { inhSeen.push(i + ":" + v); return true; });
console.log("inheritedSeen=" + inhSeen.join("|") + " out=" + JSON.stringify(inhOut) + " keys=" + Object.keys(inh).join(","));

// reduce without a seed starts from the first present element, not index 0.
const rs: any[] = [];
rs.length = 7;
rs[3] = 4;
rs[6] = 7;
const rsSeen: string[] = [];
const rsResult = rs.reduce((acc, value, index) => { rsSeen.push(index + ":" + acc + ":" + value); return acc + value; });
console.log("sparseReduce=" + rsResult + " seen=" + rsSeen.join("|"));

// Callbacks observe holes filled before their visit, and skip indices deleted before theirs.
const mut = [1, , 3, 4] as any[];
const mutSeen: string[] = [];
const mutSum = mut.reduce((acc, v, i) => {
  mutSeen.push(i + ":" + v);
  if (i === 0) mut[1] = 2;
  if (i === 1) delete mut[3];
  return acc + v;
}, 0);
console.log("holeMutationSum=" + mutSum + " seen=" + mutSeen.join(",") + " keys=" + Object.keys(mut).join(","));
const hm = [0, , 2, , 4] as any[];
const hmSeen: string[] = [];
hm.forEach((v, i) => {
  hmSeen.push(i + "=" + v);
  if (i === 0) hm[1] = 11;
  if (i === 2) delete hm[4];
});
console.log("holeMutationForEach=" + hmSeen.join(",") + " keys=" + Object.keys(hm).join(","));
console.log("holeMutationMap=" + hm.map((v, i) => i + ":" + v).join("|") + " join=" + hm.join("-"));
console.log("holeMutationHas=" + (1 in hm) + "," + (3 in hm) + "," + (4 in hm));
