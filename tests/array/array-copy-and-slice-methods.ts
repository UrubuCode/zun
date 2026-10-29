// Copying and reordering methods: slice, concat (with Symbol.isConcatSpreadable),
// fill, copyWithin (including overlapping ranges), at, reverse, and the ES2023
// non-mutating copies toReversed, toSorted, toSpliced and with. The focus is on
// argument coercion, shared references, holes, and which methods materialise them.

const five = () => [0, 1, 2, 3, 4];
const eight = () => [0, 1, 2, 3, 4, 5, 6, 7];
const holeMask = (a: any[], n: number) => Array.from({ length: n }, (_, i) => (i in a ? "y" : "n")).join("");

// slice: a copy; negative indices count from the end; holes are preserved.
const a1 = five();
console.log("sliceFrom2=" + a1.slice(2).join(",") + " range=" + a1.slice(1, 3).join(",") + " neg=" + a1.slice(-2).join(","));
console.log("sliceOriginal=" + a1.join(",") + " sliceIdentity=" + (a1.slice() === a1));
const sh: any[] = [0, , 2];
console.log("sliceHoles=" + holeMask(sh.slice(0, 3), 3));

// slice coerces each argument with ToIntegerOrInfinity: NaN is 0, fractions truncate,
// strings and booleans convert, and objects go through ToPrimitive first.
function sl(label: string, ...args: any[]) { console.log(label + "=" + (five().slice as any)(...args).join(",") + "|"); }
sl("sNone");
sl("sZero", 0);
sl("sNeg2", -2);
sl("sNeg99", -99);
sl("sPos99", 99);
sl("sNaN", NaN);
sl("sUndef", undefined);
sl("sNull", null);
sl("sTrue", true);
sl("sFrac", 1.9);
sl("sNegFrac", -1.9);
sl("sStr", "2");
sl("sStrNeg", "-2");
sl("sStrJunk", "abc");
sl("sEmpty", "");
sl("sRange", 1, 3);
sl("sNegRange", -3, -1);
sl("sCrossed", 3, 1);
sl("sEndUndef", 1, undefined);
sl("sEndNull", 1, null);
sl("sInf", -Infinity, Infinity);
const box = { valueOf() { return 2; } };
console.log("sObjStart=" + five().slice(box as any).join(","));
const boxStr = { toString() { return "-2"; } };
console.log("sObjStrStart=" + five().slice(boxStr as any).join(","));
const order: string[] = [];
five().slice({ valueOf() { order.push("start"); return 1; } } as any, { valueOf() { order.push("end"); return 3; } } as any);
console.log("sCoercionOrder=" + order.join(","));
let read = 0;
const watched: any = new Proxy([1, 2, 3], { get(t: any, k) { if (typeof k === "string" && /^\d+$/.test(k)) read++; return t[k]; } });
try { Array.prototype.slice.call(watched, { valueOf() { throw new RangeError("stop"); } } as any); }
catch (e: any) { console.log("sAbort=" + e.name + " elementsRead=" + read); }
const like: any = { length: 3, 0: "a", 2: "c" };
const genericSlice = Array.prototype.slice.call(like, 0);
console.log("sGenericIsArr=" + Array.isArray(genericSlice) + " in=" + holeMask(genericSlice, 3));

// concat: spreads arrays one level and preserves their holes; anything else is
// appended whole unless it opts in with Symbol.isConcatSpreadable.
const h: any[] = [1, , 3];
const cr = h.concat([4, , 6]);
console.log("concatLen=" + cr.length + " in=" + holeMask(cr, 6) + " v=" + cr.map(String).join(","));
console.log("concatOneLevel=" + JSON.stringify([1].concat([[2, 3]] as any)));
console.log("concatMixed=" + JSON.stringify(([1] as any).concat(2, [3, 4], [[5]])));
console.log("concatArrayLike=" + JSON.stringify(([0] as any).concat({ length: 2, 0: "a" })));
console.log("concatString=" + JSON.stringify(([0] as any).concat("ab")));
const forced: any = { length: 2, 0: "x", 1: "y" };
forced[Symbol.isConcatSpreadable] = true;
console.log("concatForced=" + JSON.stringify(([0] as any).concat(forced)));
const short: any = { length: 1, 0: "s", [Symbol.isConcatSpreadable]: true };
console.log("concatShort=" + JSON.stringify(([] as any).concat(short)));
const blocked: any = [1, 2];
blocked[Symbol.isConcatSpreadable] = false;
const bl = ([0] as any).concat(blocked);
console.log("concatBlocked=" + bl.length + " isArr=" + Array.isArray(bl[1]));
const zeroFlag: any = [7];
zeroFlag[Symbol.isConcatSpreadable] = 0;
console.log("concatZeroFlag=" + ([0] as any).concat(zeroFlag).length);
const copy = h.concat();
console.log("concatNoArgs=" + holeMask(copy, 3) + " same=" + (copy === h));
const cx: any = [1, 2];
const cy: any = [3, 4];
console.log("concatSpreadEdges=" + ([] as any).concat(cx, cy).length + " " + JSON.stringify(([] as any).concat([])));
console.log("concatOriginal=" + [1, 2].concat(3, 4).join(","));

// fill: writes one reference into every slot; its three indices clamp like slice.
const objs = new Array(3).fill({ n: 0 });
objs[0].n = 9;
console.log("fillShared=" + objs.map((o: any) => o.n).join(",") + " same=" + (objs[0] === objs[2]));
console.log("fillAll=" + five().fill(9).join(",") + " from2=" + five().fill(9, 2).join(","));
console.log("fillNeg=" + five().fill(9, -2).join(",") + " range=" + five().fill(9, 2, 4).join(","));
console.log("fillNegRange=" + five().fill(9, -3, -1).join(",") + " startGtEnd=" + five().fill(9, 4, 2).join(","));
console.log("fillBeyond=" + five().fill(9, 10, 20).join(",") + " nan=" + five().fill(9, NaN, NaN).join(","));
console.log("fillUndefEnd=" + five().fill(9, 1, undefined).join(",") + " frac=" + five().fill(9, 1.7, 3.9).join(","));
const holes: any[] = [1, , 3, , 5];
holes.fill(0, 1, 4);
console.log("fillHoles=" + holeMask(holes, 5) + " v=" + holes.map(String).join(","));
const fo = [1, 2];
console.log("fillIdentity=" + (fo.fill(0) === fo));
const fl: any = { length: 3 };
Array.prototype.fill.call(fl, "z", 1);
console.log("fillGeneric=" + [0, 1, 2].map((i) => String(fl[i])).join(","));
try { Object.freeze([1, 2]).fill(0); } catch (e: any) { console.log("fillFrozen=" + e.name); }

// copyWithin: copies in whichever direction preserves the source when the ranges overlap.
console.log("cwForward=" + eight().copyWithin(0, 3).join(","));
console.log("cwBackward=" + eight().copyWithin(3, 0).join(","));
console.log("cwBackBounded=" + eight().copyWithin(3, 0, 5).join(","));
console.log("cwSelf=" + eight().copyWithin(2, 2).join(","));
console.log("cwShiftDown=" + eight().copyWithin(1, 0).join(","));
console.log("cwShiftUp=" + eight().copyWithin(0, 1).join(","));
console.log("cwNegTarget=" + eight().copyWithin(-3, 0, 3).join(","));
console.log("cwNegStart=" + eight().copyWithin(0, -3).join(","));
console.log("cwNegEnd=" + eight().copyWithin(0, 1, -1).join(","));
console.log("cwAllNeg=" + eight().copyWithin(-2, -4, -2).join(","));
console.log("cwOutOfRange=" + eight().copyWithin(20, 0).join(","));
console.log("cwStartGtEnd=" + eight().copyWithin(0, 5, 2).join(","));
console.log("cwFrac=" + eight().copyWithin(0.9, 3.9).join(","));
const a2 = [1, 2, 3, 4, 5];
console.log("cwNeg2=" + a2.slice().copyWithin(-2).join(",") + " neg3m1=" + a2.slice().copyWithin(-2, -3, -1).join(","));
console.log("cwNoop=" + a2.slice().copyWithin(0, 0).join(",") + " mid=" + a2.slice().copyWithin(1, -2, -1).join(","));
const ch: any[] = [0, , 2, 3, 4];
ch.copyWithin(3, 0, 2);
console.log("cwHoles=" + holeMask(ch, 5) + " v=" + ch.map(String).join(","));
const co = [1, 2, 3];
console.log("cwIdentity=" + (co.copyWithin(0, 1) === co) + " len=" + co.length);
console.log("cwTyped=" + Array.from(new Uint8Array([1, 2, 3, 4, 5]).copyWithin(0, 2)).join(","));

// at: a relative index; fractions truncate, NaN and -0 are 0, and out-of-range is undefined.
const at5 = [10, 20, 30, 40, 50];
console.log("atFrac=" + at5.at(1.9) + " atNegFrac=" + at5.at(-2.7) + " atNeg1=" + at5.at(-1) + " atNeg2=" + at5.at(-2));
console.log("atOor=" + at5.at(5) + " atNeg6=" + at5.at(-6) + " atNaN=" + at5.at(NaN) + " atNegZero=" + at5.at(-0));
console.log("atStr=" + at5.at("2" as any) + " atTrue=" + at5.at(true as any) + " atUndef=" + at5.at(undefined as any));
const abox = { valueOf() { return 2; } };
console.log("atObj=" + at5.at(abox as any) + " atEmpty=" + String([].at(0)) + "," + String([].at(-1)));
console.log("atGeneric=" + Array.prototype.at.call(like, -1) + " atHole=" + String(sh.at(1)) + " atSparseLen=" + [, "x"].at(-1));
const at3 = ["a", "b", "c"];
console.log("atSimple=" + at3.at(1) + " " + at3.at(-1) + " " + at3.at(-2) + " " + at3.at(0) + " " + at3.at(9));

// reverse: in place and returns the same array; holes move with the values.
const rv = [1, 2, 3];
console.log("reverseIdentity=" + (rv.reverse() === rv) + " len=" + rv.length + " v=" + rv.join(","));
const rh: any[] = [0, , 2, , 4];
rh.reverse();
console.log("reverseHoles=" + holeMask(rh, 5) + " v=" + rh.map(String).join(","));
const rodd: any[] = [0, , 2];
rodd.reverse();
console.log("reverseOdd=" + holeMask(rodd, 3) + " v=" + rodd.map(String).join(","));
const rone: any[] = [, ];
rone.reverse();
console.log("reverseHoleOnly=" + rone.length + " in0=" + (0 in rone));
const rl: any = { length: 3, 0: "a", 2: "c" };
Array.prototype.reverse.call(rl);
console.log("reverseArrayLike=" + [0, 1, 2].map((i) => (i in rl ? String(rl[i]) : "hole")).join(","));
try { Array.prototype.reverse.call("abc" as any); } catch (e: any) { console.log("reverseString=" + e.name); }

// ES2023 copies: never mutate, read holes as undefined through Get, and create
// plain arrays (no species).
const src: any[] = [1, , 3, , 5];
console.log("srcHoles=" + holeMask(src, 5));
const tr = src.toReversed();
console.log("toReversed=" + holeMask(tr, 5) + " " + tr.map((v) => String(v)).join(","));
const tw = src.with(1, 99);
console.log("with=" + holeMask(tw, 5) + " " + tw.map((v) => String(v)).join(","));
console.log("withNeg=" + src.with(-1, "Z").map((v) => String(v)).join(","));
try { src.with(5, 0); } catch (e: any) { console.log("withOob=" + e.name); }
try { src.with(-6, 0); } catch (e: any) { console.log("withNegOob=" + e.name); }
const tsp = src.toSpliced(1, 2, "a", "b", "c");
console.log("toSpliced=" + tsp.length + " " + tsp.map((v) => String(v)).join(","));
console.log("toSplicedCount=" + src.toSpliced(2).map((v) => String(v)).join(",") + " none=" + src.toSpliced().length);
console.log("srcAfter=" + src.length + " " + holeMask(src, 5));

const sortIn = [3, 1, 4, 1, 5];
console.log("toReversedLit=" + sortIn.toReversed().join(",") + " original=" + sortIn.join(","));
console.log("toSortedDesc=" + sortIn.toSorted((a, b) => b - a).join(",") + " original2=" + sortIn.join(","));
console.log("toSplicedLit=" + sortIn.toSpliced(1, 2, 9, 9).join(",") + " original3=" + sortIn.join(","));
const w1 = [1, 2, 3, 4, 5];
console.log("withLit=" + w1.with(2, 99).join(",") + " original=" + w1.join(",") + " negWith=" + w1.with(-1, 88).join(",") + " firstWith=" + w1.with(0, 11).join(","));
