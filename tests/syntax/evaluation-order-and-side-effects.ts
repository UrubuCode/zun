// Evaluation order and side-effect counts. Each probe records what ran and in
// which order, so a reordering or a double evaluation changes a printed line.

const log: string[] = [];
function mark<T>(label: string, value: T): T {
  log.push(label);
  return value;
}
function flush(): string {
  const s = log.join(",");
  log.length = 0;
  return s;
}

// --- computed member assignment: base, key, then value ---
const target: any = {};
function baseOf(): any { log.push("base"); return target; }
function keyOf(): string { log.push("key"); return "x"; }
function valOf(): number { log.push("val"); return 7; }
baseOf()[keyOf()] = valOf();
console.log("computed_assign=" + target.x + "|" + flush());

// --- the reference of an assignment target is taken before the right side ---
let o: any = { name: "original" };
const original = o;
const replacement: any = { name: "replacement" };
o.x = o = replacement;
console.log("ref_before_rhs=" + (original.x === replacement) + "|" + ("x" in o) + "|" + (o === replacement));

// --- chained assignment: targets resolve left to right, the value is computed once ---
let ca = 0, cb = 0, cc = 0;
const chainResult = (ca = cb = cc = 7);
console.log("chain_values=" + ca + "," + cb + "," + cc + "|" + chainResult);
let rhsCalls = 0;
function source(): number { rhsCalls++; return 42; }
let p = 0, q = 0;
p = q = source();
console.log("chain_rhs_once=" + p + "," + q + "|calls=" + rhsCalls);
const holder: any = { first: null, second: null };
function objFor(label: string): any { log.push("ref:" + label); return holder; }
objFor("outer").first = objFor("inner").second = mark("value", "V");
console.log("chain_ref_order=" + flush() + "|" + holder.first + holder.second);
const t1: any = {}, t2: any = {};
t1[mark("k1", "a")] = t2[mark("k2", "b")] = mark("v", 9);
console.log("chain_keys=" + flush() + "|" + t1.a + "/" + t2.b);
let tail = "";
const chainSetter: any = { set s(v: string) { tail = "via-setter:" + v; } };
let plain = "";
plain = chainSetter.s = "shared";
console.log("chain_through_setter=" + plain + "|" + tail);

// --- self-assignment and index-before-value ---
let si = 0;
si = si++;
console.log("self_postfix=" + si);
let sj = 0;
sj = ++sj;
console.log("self_prefix=" + sj);
const idxArr: any[] = [];
let idxN = 0;
idxArr[idxN++] = idxN;
console.log("index_before_value=" + JSON.stringify(idxArr) + "|n=" + idxN);
let mx = { x: 1 };
mx.x += (mx.x = 10);
console.log("compound_reads_first=" + mx.x);

// --- compound assignment reads and writes its member target once ---
let targetCalls = 0, keyCalls = 0;
const ct: any = { x: 5 };
const getT = (): any => { targetCalls++; return ct; };
const getK = (): string => { keyCalls++; return "x"; };
getT()[getK()] += 7;
console.log("compound_once=" + ct.x + "|" + targetCalls + "|" + keyCalls);

const store: any = { a: { n: 1 }, arr: [10, 20] };
let calls = 0;
function pega(): any { calls++; return store.a; }
function idx(): number { calls++; return 1; }
pega().n += 5;
console.log("base_once=" + store.a.n + "|" + calls);
store.arr[idx()] *= 3;
console.log("key_once=" + store.arr[1] + "|" + calls);
pega().n++;
console.log("incdec_base=" + store.a.n + "|" + calls);
store.arr[idx()]--;
console.log("incdec_key=" + store.arr[1] + "|" + calls);
function obj(): any { calls++; return store; }
obj().arr[idx()] += 100;
console.log("base_and_key=" + store.arr[1] + "|" + calls);
console.log("compound_value=" + (pega().n += 1) + "|" + calls);

// --- compound and increment chains over nested members ---
const deep: any = { a: { b: 1, arr: [10, 20] }, k: "b" };
deep.a.b += 5;
deep.a.arr[1] *= 3;
deep.a[deep.k] |= 8;
deep.a.arr[0] -= 4;
deep.a.b <<= 2;
console.log("member_chain=" + deep.a.b + "|" + deep.a.arr.join(","));
const inc: any = { a: { n: 1, arr: [5] } };
inc.a.n++;
++inc.a.n;
inc.a.arr[0]--;
const postVal = inc.a.n++;
const preVal = ++inc.a.n;
console.log("incdec_chain=" + inc.a.n + "|" + postVal + "|" + preVal + "|" + inc.a.arr[0]);

// --- logical assignment: the property is read once, written only when needed ---
const lg: any = {
  _x: 0,
  get x() { log.push("get:" + this._x); return this._x; },
  set x(v: any) { log.push("set:" + v); this._x = v; },
};
lg.x ||= 5;
lg.x &&= 7;
lg.x ??= 9;
console.log("logical_getter=" + lg._x + "|" + flush());
let lrhs = 0;
const lrhsFn = (): number => { lrhs++; return 9; };
let la: any = 0, lb: any = 2, lc: any = null;
la &&= lrhsFn();
lb ||= lrhsFn();
lc ??= lrhsFn();
console.log("logical_rhs=" + la + "," + lb + "," + lc + "|calls=" + lrhs);

// --- short-circuit operators skip the operand they do not need ---
function side(tag: string, val: any): any { log.push(tag); return val; }
const sc1 = side("A", false) && side("B", true);
console.log("and_skip=" + sc1 + "|" + flush());
const sc2 = side("C", true) || side("D", false);
console.log("or_skip=" + sc2 + "|" + flush());
const sc3 = side("E", null) ?? side("F", 9);
console.log("nullish_rhs=" + sc3 + "|" + flush());
const sc4 = side("G", 0) ?? side("H", 5);
console.log("nullish_keeps_zero=" + sc4 + "|" + flush());

// --- call arguments left to right, after the callee ---
function t(x: number): number { log.push("t" + x); return x; }
function f3(a: number, b: number, c: number): number { return a * 100 + b * 10 + c; }
console.log("args=" + f3(t(1), t(2), t(3)) + "|" + flush());
function callee(): (a: number, b: number) => number {
  log.push("callee");
  return (a: number, b: number) => a - b;
}
console.log("callee_first=" + callee()(t(7), t(4)) + "|" + flush());

// --- an optional call with a nullish callee never evaluates its arguments ---
let optCalls = 0;
const optArg = (): number => { optCalls++; return 5; };
const absent: any = null;
const present: any = (x: number) => x * 2;
console.log("absent_call=" + absent?.(optArg()) + "|" + optCalls);
console.log("present_call=" + present?.(optArg()) + "|" + optCalls);

// --- array spread operands and object literal keys/values in source order ---
function part(name: string, values: number[]): number[] { log.push(name); return values; }
const spreadOut = [...part("a", [1, 2]), 3, ...part("b", [4, 5])];
console.log("spread_values=" + spreadOut.join(",") + "|" + flush());
function key(n: string): string { log.push("k" + n); return n; }
function value(n: number): number { log.push("v" + n); return n; }
const ordered = { [key("a")]: value(1), fixed: value(2), [key("b")]: value(3) };
console.log("object_order=" + JSON.stringify(ordered) + "|" + flush());
