// Mutators at their boundaries: push, pop, shift and unshift (return values,
// length changes, holes at the ends, generic array-likes, overflow, frozen
// arrays) and splice (argument clamping, removed-element results, holes).

// Stack and queue basics, with the return value of each call.
const arr = [1, 2, 3];
const len1 = arr.push(4);
console.log("push=" + arr.join(",") + " len=" + len1);
const popped = arr.pop();
console.log("pop=" + popped + " arr=" + arr.join(","));
const shifted = arr.shift();
console.log("shift=" + shifted + " arr=" + arr.join(","));
const len2 = arr.unshift(0);
console.log("unshift=" + arr.join(",") + " len=" + len2);
arr.push(5, 6);
console.log("pushMulti=" + arr.join(","));
arr.unshift(-2, -1);
console.log("unshiftMulti=" + arr.join(","));
console.log("unshiftReturn=" + [3, 4].unshift(2) + " " + [3, 4].unshift(0, 1) + " " + ([] as number[]).unshift(1));

// Empty arrays: pop and shift answer undefined and leave length at 0.
const empty: any[] = [];
console.log("popEmpty=" + String(empty.pop()) + " len=" + empty.length);
console.log("shiftEmpty=" + String(empty.shift()) + " len=" + empty.length);
console.log("pushNone=" + [1, 2].push() + " pushMany=" + [].push(1, 2, 3) + " unshiftNone=" + [1, 2].unshift() + " unshiftMany=" + [3].unshift(1, 2));

// Return types: push returns a number; pop and shift return the element.
const rt: any[] = [];
console.log("returnTypes=" + typeof rt.push(1) + "," + typeof rt.pop() + "," + typeof rt.shift());

// Holes at the ends: unshift moves holes with the elements, shift pulls the
// first element down, pop on a trailing hole answers undefined and shortens.
const uh: any[] = [1, , 3];
uh.unshift("a");
console.log("unshiftHoles=" + [0, 1, 2, 3].map((i) => (i in uh ? "y" : "n")).join("") + " v=" + uh.map(String).join(","));
const cuh: any[] = [, , "x"];
cuh.unshift("a", "b");
console.log("unshiftHoleLen=" + cuh.length + " join=" + cuh.join("|") + " keys=" + Object.keys(cuh).join(","));
console.log("unshiftHoleIn=" + (2 in cuh) + "," + (3 in cuh) + "," + (4 in cuh));
const sh: any[] = [, 2, , 4];
const firstOut = sh.shift();
console.log("shiftHole=" + String(firstOut) + " len=" + sh.length + " in=" + [0, 1, 2].map((i) => (i in sh ? "y" : "n")).join(""));
const pHole: any[] = [1, , ];
console.log("popHole=" + String(pHole.pop()) + " len=" + pHole.length);

// Generic mutators on array-likes: length is read with ToLength and updated.
const like: any = { length: 2, 0: "a", 1: "b" };
console.log("genericPush=" + Array.prototype.push.call(like, "c") + " len=" + like.length + " v=" + like[2]);
console.log("genericPop=" + Array.prototype.pop.call(like) + " len=" + like.length + " has2=" + (2 in like));
const noLen: any = {};
console.log("noLenPush=" + Array.prototype.push.call(noLen, "x") + " len=" + noLen.length + " v=" + noLen[0]);
const strLen: any = { length: "1", 0: "a" };
Array.prototype.push.call(strLen, "b");
console.log("strLenPush=" + strLen.length + " v=" + strLen[1] + " type=" + typeof strLen.length);
const huge: any = { length: Number.MAX_SAFE_INTEGER };
try { Array.prototype.push.call(huge, "x"); } catch (e: any) { console.log("pushOverflow=" + e.name); }

// Frozen arrays refuse length changes with a TypeError.
const frozen = Object.freeze([1, 2]);
try { frozen.pop(); } catch (e: any) { console.log("frozenPop=" + e.name); }
console.log("frozenLen=" + frozen.length);

// A very sparse array: shift walks it without materialising the slots.
const sparse: any = [];
sparse[3] = "d";
console.log("sparseShift=" + String(sparse.shift()) + " len=" + sparse.length + " keys=" + Object.keys(sparse).join(","));

// splice: the return value is the removed slice, the array is edited in place.
const s1 = [1, 2, 3, 4, 5];
const removed = s1.splice(2, 2);
console.log("spliceRemoved=" + removed.join(",") + " arr=" + s1.join(","));
const s2 = [1, 2, 5];
s2.splice(2, 0, 3, 4);
console.log("spliceInsert=" + s2.join(","));
const s3 = [1, 2, 3, 4, 5];
s3.splice(1, 2, 8, 9);
console.log("spliceReplace=" + s3.join(","));
const grow = [1, 2];
const growRet = grow.splice(1, 0, "a", "b", "c");
console.log("spliceGrow=" + grow.join(",") + " retLen=" + growRet.length + " retIsArr=" + Array.isArray(growRet));

// splice clamps start and deleteCount: negatives count from the end, NaN is 0,
// oversized counts are capped, and a missing deleteCount removes the tail.
function show(label: string, ...args: any[]) {
  const a = [0, 1, 2, 3, 4];
  const r = (a.splice as any)(...args);
  console.log(label + " removed=[" + r.map(String).join(",") + "] rest=[" + a.map(String).join(",") + "] len=" + a.length);
}
show("start2", 2);
show("startNeg2", -2);
show("startNeg99", -99);
show("start99", 99);
show("startNaN", NaN);
show("startUndef", undefined);
show("startFrac", 1.9);
show("startStr", "2", "1");
show("none");
show("countNeg5", 1, -5);
show("count99", 1, 99);
show("countNaN", 1, NaN);
show("count0Ins", 1, 0, "a", "b");
show("neg1Ins", -1, 1, "x");

// Holes in the removed range stay holes in the removed array; the remainder
// keeps its own holes too.
const h: any[] = [1, , 3, , 5];
const rem = h.splice(1, 3);
console.log("remIn=" + [0, 1, 2].map((i) => (i in rem ? "y" : "n")).join("") + " rem=" + rem.map(String).join(","));
console.log("leftIn=" + [0, 1].map((i) => (i in h ? "y" : "n")).join("") + " left=" + h.map(String).join(","));
