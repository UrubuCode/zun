// Loops (for, while, do-while, for-in, for-of heads), labelled break/continue,
// switch dispatch, and the scope of a switch body.

// --- for loops and per-iteration let bindings ---
function sumRange(limit: number): number {
  let total = 0;
  for (let i = 0; i <= limit; i++) total += i;
  return total;
}
console.log("sum_range=" + sumRange(3) + "|" + sumRange(5));

const letFns: Array<() => number> = [];
for (let i = 0; i < 3; i++) letFns.push(() => i);
console.log("let_closures=" + letFns.map((f) => f()).join(","));
const varFns: Array<() => number> = [];
for (var v = 0; v < 3; v++) varFns.push(() => v);
console.log("var_closures=" + varFns.map((f) => f()).join(",") + "|after=" + v);
const early: Array<() => number> = [];
for (let i = 10; i < 13; i++) early.push(() => i);
console.log("closure_before_update=" + early.map((f) => f()).join(","));
const mutated: number[] = [];
for (let i = 0; i < 3; i++) {
  const f = () => i;
  i += 10;
  mutated.push(f());
}
console.log("body_mutation=" + mutated.join(","));
const pairs: Array<() => string> = [];
for (let i = 0, j = 10; i < 3; i++, j--) pairs.push(() => i + ":" + j);
console.log("two_declarators=" + pairs.map((f) => f()).join("|"));
const seenByCond: number[] = [];
for (let i = 0; seenByCond.push(i) && i < 2; i++) { /* the test records the counter */ }
console.log("cond_sees=" + seenByCond.join(","));
const fromUpdate: Array<() => number> = [];
for (let i = 0; i < 3; fromUpdate.push(() => i), i++) { /* update captures copies */ }
console.log("from_update=" + fromUpdate.map((f) => f()).join(","));
const beforeBreak: Array<() => number> = [];
for (let i = 0; i < 5; i++) {
  beforeBreak.push(() => i);
  if (i === 2) break;
}
console.log("before_break=" + beforeBreak.map((f) => f()).join(","));
const withContinue: Array<() => number> = [];
for (let i = 0; i < 4; i++) {
  if (i === 1) continue;
  withContinue.push(() => i);
}
console.log("with_continue=" + withContinue.map((f) => f()).join(","));
const nestedFns: Array<() => string> = [];
for (let a = 0; a < 2; a++) for (let b = 0; b < 2; b++) nestedFns.push(() => a + "" + b);
console.log("nested_let=" + nestedFns.map((f) => f()).join(","));
const bodyLet: Array<() => number> = [];
for (let i = 0; i < 3; i++) {
  let doubled = i * 2;
  bodyLet.push(() => doubled);
  doubled = i * 2 + 1;
}
console.log("body_let=" + bodyLet.map((f) => f()).join(","));
const varBody: Array<() => number> = [];
for (let i = 0; i < 3; i++) {
  var shared = i;
  varBody.push(() => shared);
}
console.log("var_body=" + varBody.map((f) => f()).join(",") + "|shared=" + shared);
console.log("loop_binding_leaked=" + typeof (globalThis as any).i);
const bumped: number[] = [];
for (let i = 0; i < 6; i++) {
  const bump = () => { i += 1; };
  bumped.push(i);
  bump();
}
console.log("bump_in_closure=" + bumped.join(","));

// --- while and do-while: the body runs before the first test; continue goes to the test ---
const onceDo: string[] = [];
do { onceDo.push("ran"); } while (false);
console.log("do_runs_once=" + onceDo.join(","));
let dn = 0;
const dsteps: number[] = [];
do { dn += 1; dsteps.push(dn); } while (dn < 4);
console.log("do_sees_body=" + dsteps.join(",") + "|final=" + dn);
let dc = 0;
const dvisited: string[] = [];
do {
  dc += 1;
  if (dc % 2 === 1) continue;
  dvisited.push("even" + dc);
} while (dc < 6);
console.log("do_continue=" + dvisited.join(",") + "|c=" + dc);
let db = 0;
const dtested: string[] = [];
do {
  db += 1;
  if (db === 2) break;
} while (dtested.push("t" + db) > 0);
console.log("do_break_skips_test=" + dtested.join(",") + "|b=" + db);
let dRet = 0;
function doReturn(): string {
  const dlog: string[] = [];
  do {
    dRet += 1;
    if (dRet === 2) return dlog.join(",") + "|left-at=" + dRet;
    dlog.push("pass" + dRet);
  } while (dlog.push("cond" + dRet) > 0);
  return "unreachable";
}
console.log("do_return=" + doReturn());
let dLoose = "abcd";
const trimmed: string[] = [];
do {
  trimmed.push(dLoose);
  dLoose = dLoose.slice(1);
} while (dLoose);
console.log("do_truthy=" + trimmed.join(",") + "|left=" + JSON.stringify(dLoose));
let ds = 1;
do ds *= 3;
while (ds < 50);
console.log("do_single_stmt=" + ds);
let dd = 0;
if (true) do dd += 5; while (dd < 12);
console.log("do_as_if_body=" + dd);
const grid: string[] = [];
let oi = 0;
do {
  let ij = 0;
  do {
    grid.push(oi + "" + ij);
    ij += 1;
  } while (ij < 2);
  oi += 1;
} while (oi < 2);
console.log("do_nested=" + grid.join(","));
const lbDo: string[] = [];
let li = 0;
outerDo: do {
  li += 1;
  let lj = 0;
  do {
    lj += 1;
    if (lj === 2) continue outerDo;
    lbDo.push(li + ":" + lj);
  } while (lj < 4);
  lbDo.push("inner-finished" + li);
} while (li < 3);
console.log("do_labelled_continue=" + lbDo.join(","));
let wg = 0;
const wSaw: number[] = [];
while ((wg = wg + 1) < 4) wSaw.push(wg);
console.log("while_assign=" + wSaw.join(",") + "|wg=" + wg);
let wpost = 0;
const wpostSaw: number[] = [];
while (wpost++ < 3) wpostSaw.push(wpost);
console.log("while_postfix=" + wpostSaw.join(",") + "|" + wpost);
const queue = [3, 2, 1, 0, "", null];
let item: any;
const drained: string[] = [];
let qi = 0;
while ((item = queue[qi++])) drained.push(String(item));
console.log("while_truthy_drain=" + drained.join(",") + "|stopped_at=" + qi);
let cnt = 0;
const comma: string[] = [];
let cx = 0, cy = 10;
while ((cx = cx + 1, cy = cy - 1, cx < cy) && cnt++ < 10) comma.push(cx + "/" + cy);
console.log("while_comma=" + comma.join(" ") + "|cx=" + cx + "|cy=" + cy);

// --- for-in and for-of heads: keys, values, patterns and bindings ---
const forInKeys: string[] = [];
const parentObj: any = { inherited: true };
const childObj: any = Object.create(parentObj);
childObj.own1 = 1;
childObj.own2 = 2;
Object.defineProperty(childObj, "hidden", { value: 99, enumerable: false });
for (const k in childObj) forInKeys.push(k);
console.log("for_in_chain=" + forInKeys.join(","));
const ownOnly: string[] = [];
for (const k in childObj) if (Object.prototype.hasOwnProperty.call(childObj, k)) ownOnly.push(k);
console.log("for_in_own=" + ownOnly.join(","));
const arrKeys: string[] = [];
for (const k in ["a", "b"]) arrKeys.push(typeof k + ":" + k);
console.log("for_in_array_keys=" + arrKeys.join(","));
const mapForIn: string[] = [];
for (const k in new Map([["a", 1]])) mapForIn.push(k);
console.log("for_in_map=" + mapForIn.length);
const delObj: any = { a: 1, b: 2, c: 3 };
const visited: string[] = [];
for (const k in delObj) {
  visited.push(k);
  if (k === "a") delete delObj.b;
}
console.log("for_in_delete=" + visited.includes("a") + "|" + visited.includes("c"));
const keysOfPattern: string[] = [];
for (const [c0, c1] of Object.keys({ ab: 1, cd: 2 })) keysOfPattern.push(c1 + c0);
console.log("for_of_key_pattern=" + keysOfPattern.join(","));
const inPattern: string[] = [];
for (const [f0] in { xy: 1 }) inPattern.push(f0);
console.log("for_in_pattern=" + inPattern.join(","));
const headPairs: any[] = [[1, "a"], [2, "b"], [3, "c"]];
const headSeen: string[] = [];
for (const [n, s] of headPairs) headSeen.push(n + s);
console.log("for_of_pattern=" + headSeen.join(","));
const constClosures: Array<() => string> = [];
for (const [n, s] of headPairs) constClosures.push(() => n + s);
console.log("const_head_closures=" + constClosures.map((f) => f()).join(","));
function varHead(): string {
  const fns: Array<() => string> = [];
  for (var [vn, vs] of headPairs) fns.push(() => vn + vs);
  return fns.map((f) => f()).join(",") + "|after=" + vn + vs;
}
console.log("var_head=" + varHead());
let outerN = 0;
let outerS = "";
for ([outerN, outerS] of headPairs) { /* the head assigns existing bindings */ }
console.log("assign_head=" + outerN + outerS);
const sink: any = { last: null, key: null };
for ([sink.key, sink.last] of headPairs) { /* member targets are written each round */ }
console.log("member_head=" + sink.key + sink.last);
let counter = 0;
function nextId(): number { counter += 1; return counter; }
const withHoles: any[] = [{ id: 100 }, {}, {}, { id: 200 }];
const idSeen: number[] = [];
for (const { id = nextId() } of withHoles) idSeen.push(id);
console.log("head_default_each=" + idSeen.join(",") + "|calls=" + counter);
const rests: any[] = [];
for (const [, ...tail] of headPairs) rests.push(tail);
console.log("head_rest=" + rests.map((r) => "[" + r.join("") + "]").join("") + "|distinct=" + (rests[0] !== rests[1]));
const holeSeen: string[] = [];
for (const [, , third] of [[1, 2, 3], [4, 5, 6]]) holeSeen.push(String(third));
console.log("head_holes=" + holeSeen.join(","));
const cfgs: any[] = [{ opts: { deep: 1 } }, { opts: {} }, {}];
const cfgSeen: string[] = [];
for (const { opts: { deep = -1 } = {} } of cfgs) cfgSeen.push(String(deep));
console.log("head_nested_defaults=" + cfgSeen.join(","));
const setSeen: string[] = [];
for (const [a, b] of new Set<number>([7, 8]).entries()) setSeen.push(a + "/" + b);
console.log("set_entries=" + setSeen.join(","));
const strSeen: string[] = [];
for (const [a, b] of ["xy", "zw"]) strSeen.push(a + "|" + b);
console.log("string_elements=" + strSeen.join(" "));
const mapSeen: string[] = [];
for (const [k, val2] of new Map<string, number>([["one", 1], ["two", 2]])) mapSeen.push(k + "=" + val2);
console.log("map_entries=" + mapSeen.join(","));

// --- switch: the discriminant runs once, cases in source order until a match ---
const trace: string[] = [];
function probe<T>(label: string, value: T): T {
  trace.push(label);
  return value;
}
function once(v: number): string {
  switch (probe("disc", v)) {
    case 1: return "one";
    case 2: return "two";
    case 3: return "three";
    default: return "none";
  }
}
console.log("switch_once=" + once(3) + "|" + trace.join(","));
trace.length = 0;
function stopsAtMatch(v: number): string {
  switch (v) {
    case probe("c1", 1): return "one";
    case probe("c2", 2): return "two";
    case probe("c3", 3): return "three";
    default: return "none";
  }
}
console.log("stops_at_match=" + stopsAtMatch(2) + "|" + trace.join(","));
trace.length = 0;
console.log("no_match=" + stopsAtMatch(9) + "|" + trace.join(","));
function defaultInMiddle(v: number): string {
  const out: string[] = [];
  switch (v) {
    case probe("first", 1):
      out.push("first");
      break;
    default:
      out.push("default");
      break;
    case probe("last", 2):
      out.push("last");
      break;
  }
  return out.join("+");
}
trace.length = 0;
console.log("default_middle_hit=" + defaultInMiddle(2) + "|" + trace.join(","));
trace.length = 0;
console.log("default_middle_miss=" + defaultInMiddle(9) + "|" + trace.join(","));
function fallsIntoNext(v: number): string {
  const out: string[] = [];
  switch (v) {
    case 1:
      out.push("one");
    default:
      out.push("default");
    case 2:
      out.push("two");
  }
  return out.join(",");
}
console.log("fall_1=" + fallsIntoNext(1) + "|fall_2=" + fallsIntoNext(2) + "|fall_9=" + fallsIntoNext(9));
function classify(x: number): string {
  const out: string[] = [];
  switch (x) {
    case 1: out.push("one");
    default: out.push("default");
    case 2: out.push("two"); break;
    case 3: out.push("three");
  }
  return out.join(",");
}
console.log("fall_default_between=" + [1, 2, 9, 3].map(classify).join("|"));
function noDefault(v: number): string {
  const out: string[] = ["before"];
  switch (probe("disc-nd", v)) {
    case 1:
      out.push("one");
      break;
  }
  out.push("after");
  return out.join(",");
}
trace.length = 0;
console.log("no_default=" + noDefault(5) + "|" + trace.join(","));
trace.length = 0;
switch (probe("empty-disc", 1)) {
}
console.log("empty_switch=" + trace.join(","));
function strictOnly(v: any): string {
  switch (v) {
    case probe("case-str-1", "1"): return "string-one";
    case probe("case-num-1", 1): return "number-one";
    default: return "other";
  }
}
trace.length = 0;
console.log("strict_num=" + strictOnly(1) + "|" + trace.join(","));
trace.length = 0;
console.log("strict_str=" + strictOnly("1") + "|" + trace.join(","));
const target = { k: 1 };
function byIdentity(v: any): string {
  switch (v) {
    case target: return "same-object";
    default: return "different";
  }
}
console.log("identity=" + byIdentity(target) + "|" + byIdentity({ k: 1 }));
function label(x: any): string {
  switch (x) {
    case NaN: return "case_NaN";
    case 0: return "case_0";
    case -0: return "case_-0";
    case 1: return "case_1";
    case Infinity: return "case_Infinity";
    case "1": return "case_string_1";
    case true: return "case_true";
    case null: return "case_null";
    default: return "default";
  }
}
const numericProbes: [string, any][] = [
  ["NaN", NaN], ["+0", 0], ["-0", -0], ["1", 1], ["1.0", 1.0], ["Infinity", Infinity],
  ["'1'", "1"], ["true", true], ["null", null], ["undefined", undefined], ["2", 2],
];
for (const [name, value] of numericProbes) console.log("numeric_case " + name + "=" + label(value));
function provenLabel(seed: number): string {
  let x = seed;
  x = x + 0;
  switch (x) {
    case 0: return "case_0";
    case 1: return "case_1";
    default: return "default";
  }
}
console.log("proven_numeric=" + provenLabel(-0) + "|" + provenLabel(NaN) + "|" + provenLabel(1));
function breakBelongsToSwitch(): string {
  const out: string[] = [];
  for (let i = 0; i < 3; i++) {
    switch (i) {
      case 1:
        out.push("skip" + i);
        break;
      default:
        out.push("keep" + i);
    }
    out.push("post" + i);
  }
  return out.join(",");
}
console.log("break_in_switch=" + breakBelongsToSwitch());
function unlabelledContinueFromSwitch(): string {
  const cont: string[] = [];
  for (let i = 0; i < 4; i++) {
    switch (i % 2) {
      case 0:
        continue;
      default:
        cont.push("odd" + i);
    }
    cont.push("tail" + i);
  }
  return cont.join(",");
}
console.log("continue_from_switch=" + unlabelledContinueFromSwitch());

// --- a switch body is one block: a let is in its dead zone until its case runs ---
function tdzProbe(n: number): string {
  switch (n) {
    case 0:
      try {
        return "saw:" + String(later);
      } catch (e) {
        return "tdz:" + (e as any).constructor.name;
      }
    case 1:
      let later = "declared-in-case-1";
      return "case1:" + later;
    default:
      return "default:" + typeof later;
  }
}
console.log("tdz_case0=" + tdzProbe(0) + "|tdz_case1=" + tdzProbe(1) + "|tdz_again=" + tdzProbe(0));
function fallthroughInit(n: number): string {
  const seen: string[] = [];
  switch (n) {
    case 0:
      seen.push("zero");
    case 1:
      let carried = "init@" + n;
      seen.push(carried);
    case 2:
      seen.push("two:" + carried);
      break;
    default:
      seen.push("default");
  }
  return seen.join(">");
}
console.log("fall_init_0=" + fallthroughInit(0) + "|fall_init_1=" + fallthroughInit(1));
function braced(n: number): string {
  switch (n) {
    case 0: { const v = "block-zero"; return v; }
    case 1: { const v = "block-one"; return v; }
    default: { const v = "block-default"; return v; }
  }
}
console.log("braced=" + braced(0) + "|" + braced(1) + "|" + braced(9));
function varInCase(n: number): string {
  switch (n) {
    case 0: return "case0:" + String(hoisted);
    case 1:
      var hoisted = "assigned";
      return "case1:" + hoisted;
    default: return "default:" + String(hoisted);
  }
}
console.log("var_in_case=" + varInCase(0) + "|" + varInCase(1) + "|" + varInCase(9));
const armed: string[] = [];
for (let round = 0; round < 2; round++) {
  switch (round) {
    case 0:
    case 1:
      try {
        armed.push("read:" + String(perEntry));
      } catch (e) {
        armed.push("tdz" + round);
      }
      break;
    default:
      break;
  }
  let perEntry = "never-reached";
  void perEntry;
}
console.log("tdz_rearmed=" + armed.join(","));
const shadowOuter = "outer-shared";
function discriminant(): string {
  switch (shadowOuter) {
    case "outer-shared": {
      const shadow = "inner";
      return "matched:" + shadow;
    }
    default:
      return "no-match";
  }
}
console.log("discriminant=" + discriminant());

// --- labelled break and continue, including across switch and try ---
const nestedTrace: string[] = [];
outer: for (let i = 0; i < 3; i++) {
  inner: for (let j = 0; j < 3; j++) {
    switch (i * 10 + j) {
      case 1:
        nestedTrace.push("continue_outer@" + i + j);
        continue outer;
      case 11:
        nestedTrace.push("break_inner@" + i + j);
        break inner;
      case 21:
        nestedTrace.push("break_outer@" + i + j);
        break outer;
      default:
        nestedTrace.push("v" + i + j);
        break;
    }
    nestedTrace.push("after_switch@" + i + j);
  }
  nestedTrace.push("end_i@" + i);
}
console.log("labels_across_switch=" + nestedTrace.join(","));
outerPlain: for (let i = 0; i < 3; i++) {
  for (let j = 0; j < 3; j++) {
    if (i === 1 && j === 0) continue outerPlain;
    if (i === 2 && j === 1) break outerPlain;
    nestedTrace.push("p" + i + j);
  }
}
console.log("labels_basic=" + nestedTrace.slice(-4).join(","));
const splitLoops: string[] = [];
search: for (let i = 0; i < 5; i++) {
  for (let j = 0; j < 5; j++) {
    if (i * j > 6) {
      splitLoops.push("found " + i + "," + j);
      break search;
    }
  }
}
console.log("labels_search=" + splitLoops.join(","));
const blockTrace: string[] = [];
block: {
  blockTrace.push("in");
  if (blockTrace.length === 1) break block;
  blockTrace.push("unreachable");
}
blockTrace.push("out");
console.log("labelled_block=" + blockTrace.join(","));
const sectionSeen: string[] = [];
section: {
  sectionSeen.push("start");
  {
    sectionSeen.push("inner");
    break section;
  }
  sectionSeen.push("miss");
}
sectionSeen.push("end");
console.log("labelled_nested_block=" + sectionSeen.join(","));
const wrapped: string[] = [];
wrap: {
  switch ("x") {
    case "x":
      wrapped.push("case_x");
      break wrap;
    default:
      wrapped.push("default");
  }
  wrapped.push("after_switch");
}
wrapped.push("after_wrap");
console.log("labelled_block_switch=" + wrapped.join(","));
const wh: string[] = [];
let wn = 0;
loop: while (wn < 5) {
  wn++;
  {
    switch (wn % 3) {
      case 0:
        continue loop;
      case 1:
        wh.push("one:" + wn);
        break;
      default:
        wh.push("two:" + wn);
    }
  }
  wh.push("bottom:" + wn);
}
console.log("while_label=" + wh.join(","));
const two: string[] = [];
loops: for (let i = 0; i < 4; i++) {
  guard: {
    if (i === 1) break guard;
    if (i === 3) break loops;
    two.push("kept" + i);
  }
  two.push("bottom" + i);
}
console.log("labelled_block_in_loop=" + two.join(","));
const combo: string[] = [];
keys: for (const k in { a: 0, b: 0, c: 0 }) {
  for (const v2 of [1, 2, 3]) {
    switch (k + v2) {
      case "a2":
        combo.push("skip_a");
        continue keys;
      case "b1":
        combo.push("skip_b1");
        continue;
      case "c3":
        combo.push("stop");
        break keys;
      default:
        combo.push(k + v2);
    }
  }
  combo.push("done:" + k);
}
console.log("for_in_switch_labels=" + combo.join(","));
function findPair(target2: number): string {
  search2: for (let i = 1; i <= 4; i++) {
    for (let j = 1; j <= 4; j++) {
      switch (i * j) {
        case 0:
          continue search2;
        default:
          if (i * j === target2) return "found " + i + "x" + j;
          if (i * j > target2) continue search2;
      }
    }
  }
  return "none";
}
console.log("find_pair=" + findPair(6) + "|" + findPair(9) + "|" + findPair(7) + "|" + findPair(16));
function classifyGuard(n: number): string {
  let out = "?";
  check: {
    if (n < 0) { out = "negative"; break check; }
    if (n === 0) { out = "zero"; break check; }
    switch (n % 2) {
      case 0:
        out = "even";
        break check;
      default:
        out = "odd";
    }
    out = out + "+checked";
  }
  return out;
}
console.log("guard_chain=" + [-3, 0, 4, 5].map(classifyGuard).join(","));
function pick(x: number): string {
  const hits: string[] = [];
  sw: switch (x) {
    case 0:
      hits.push("zero");
    case 1:
      hits.push("one");
      if (x === 0) break sw;
    case 2:
      hits.push("two");
      break;
    default:
      hits.push("other");
  }
  return hits.join(">");
}
console.log("labelled_switch_fall=" + pick(0) + "|" + pick(1) + "|" + pick(2) + "|" + pick(9));
const fin: string[] = [];
esc: for (let i = 0; i < 3; i++) {
  try {
    switch (i) {
      case 1:
        fin.push("break@" + i);
        break esc;
      default:
        fin.push("body" + i);
    }
  } finally {
    fin.push("finally" + i);
  }
}
console.log("finally_on_break=" + fin.join(","));
const fin2: string[] = [];
cloop: for (let i = 0; i < 3; i++) {
  try {
    if (i === 1) continue cloop;
    fin2.push("kept" + i);
  } finally {
    fin2.push("f" + i);
  }
  fin2.push("tail" + i);
}
console.log("finally_on_continue=" + fin2.join(","));
const labelName = "variable";
lonely: for (let i = 0; i < 1; i++) {
  break lonely;
}
console.log("label_not_binding=" + labelName);

// --- an obfuscator-style state machine: a switch dispatch loop over an order table ---
function flatten(input: number): number {
  const order = "3|1|4|0|2".split("|").map(Number);
  let pos = 0;
  let result = 0;
  let tmp = 0;
  while (pos < order.length) {
    switch (order[pos++]) {
      case 0: result = tmp * 2; continue;
      case 1: tmp = input + 10; continue;
      case 2: result = result - 1; continue;
      case 3: result = 0; tmp = 0; continue;
      case 4: result = tmp + result; continue;
    }
  }
  return result;
}
console.log("flatten=" + flatten(5) + "|" + flatten(0) + "|" + flatten(10));
function stateWord(x: number, y: number): string {
  let state = 0;
  let out = "";
  while (true) {
    switch (state) {
      case 0: out = ""; state = x > 0 ? 1 : 3; break;
      case 1: out += "pos"; state = y > 0 ? 2 : 4; break;
      case 2: out += "+pos"; state = 5; break;
      case 3: out += "non-pos"; state = y > 0 ? 4 : 5; break;
      case 4: out += "+non-pos"; state = 5; break;
      case 5: return out;
    }
  }
}
console.log("state_words=" + stateWord(1, 1) + "|" + stateWord(1, -1) + "|" + stateWord(-1, 1) + "|" + stateWord(-1, -1));

// --- loop conditions that mutate: the new value must be seen on the next test ---
let guardCount = 0;
function tick(): boolean { return ++guardCount < 1000; }
let m = 0;
const doSaw: number[] = [];
do { doSaw.push(m); } while ((m += 2) < 6 && tick());
console.log("cond_do=" + doSaw.join(",") + "|m=" + m);
guardCount = 0;
const cellSaw: number[] = [];
const cell = [0];
do { cellSaw.push(cell[0]); } while ((cell[0] += 2) < 6 && tick());
console.log("cond_element=" + cellSaw.join(",") + "|v=" + cell[0]);
guardCount = 0;
const forSaw: number[] = [];
for (let i = 0; (i = i + 1) <= 3 && tick(); ) forSaw.push(i);
console.log("cond_for=" + forSaw.join(","));
guardCount = 0;
let shared2 = 0;
const readers: Array<() => number> = [];
do { readers.push(() => shared2); } while ((shared2 += 3) < 9 && tick());
console.log("cond_closures=" + readers.map((f) => f()).join(",") + "|shared=" + shared2);
console.log("guard_final=" + guardCount);

// --- for-in bindings: let is fresh per key, var is one shared binding ---
const forInFns: Array<() => string> = [];
const forInObj: any = { a: 1, b: 2, c: 3 };
for (let k in forInObj) forInFns.push(() => k + forInObj[k]);
console.log("for_in_let_closures=" + forInFns.map((f) => f()).join(","));
const forInVarFns: Array<() => string> = [];
for (var q in forInObj) forInVarFns.push(() => q);
console.log("for_in_var_closures=" + forInVarFns.map((f) => f()).join(","));
