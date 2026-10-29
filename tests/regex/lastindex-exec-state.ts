// Cross-runtime: the RegExp lastIndex state machine, observed through test(),
// exec() and the String methods that drive it. Covers the plain / global /
// sticky matrix, the success and failure write-back rules, ToLength coercion on
// read, the own-property descriptor of lastIndex, and the TypeError that a
// non-writable lastIndex raises on a stateful match.

function attempt(f: () => any): string {
  try {
    const v = f();
    return v === null ? "null" : String(v);
  } catch (e: any) {
    return "!" + e.constructor.name;
  }
}

function grid(re: RegExp, subject: string, start: number): string {
  re.lastIndex = start;
  const ok = re.test(subject);
  return re.flags + " li" + start + " -> " + ok + " li" + re.lastIndex;
}

function desc(o: any, k: string): string {
  const d: any = Object.getOwnPropertyDescriptor(o, k);
  if (d === undefined) return "none";
  if ("value" in d) return "v=" + d.value + " w=" + d.writable + " e=" + d.enumerable + " c=" + d.configurable;
  return "accessor e=" + d.enumerable + " c=" + d.configurable;
}

const S = "xaxa";

// --- test() across plain, global, sticky and global+sticky ---
console.log("t-plain-0=" + grid(/a/, S, 0));
console.log("t-plain-9=" + grid(/a/, S, 9));
console.log("t-g-0=" + grid(/a/g, S, 0));
console.log("t-g-2=" + grid(/a/g, S, 2));
console.log("t-g-4=" + grid(/a/g, S, 4));
console.log("t-y-0=" + grid(/a/y, S, 0));
console.log("t-y-1=" + grid(/a/y, S, 1));
console.log("t-y-3=" + grid(/a/y, S, 3));
console.log("t-gy-1=" + grid(/a/gy, S, 1));
console.log("t-gy-0=" + grid(/a/gy, S, 0));

// --- a /g regex walks the subject across repeated calls, then wraps to 0 ---
const walkRe = /a/g;
const walk: string[] = [];
for (let i = 0; i < 4; i++) walk.push(walkRe.test(S) + ":" + walkRe.lastIndex);
console.log("walk=" + walk.join(" "));

// --- exec() writes the same slot; a plain regex never does ---
const e = /a/g;
e.lastIndex = 2;
const m1: any = e.exec(S);
console.log("exec-hit=" + m1[0] + ":" + m1.index + ":" + e.lastIndex);
const m2: any = e.exec(S);
console.log("exec-miss=" + String(m2) + ":" + e.lastIndex);
const ePlain = /a/;
ePlain.lastIndex = 3;
console.log("exec-plain=" + (ePlain.exec(S) as any).index + ":" + ePlain.lastIndex);

// --- sticky failure resets to 0 even when it started elsewhere ---
const y = /a/y;
y.lastIndex = 0;
console.log("y-fail=" + y.test(S) + ":" + y.lastIndex);
y.lastIndex = 3;
console.log("y-hit=" + y.test(S) + ":" + y.lastIndex);
y.lastIndex = 3;
const yHit: any = y.exec(S);
const afterYHit = y.lastIndex;
console.log("y-exec-hit=" + yHit[0] + ":" + afterYHit);
console.log("y-exec-miss=" + String(y.exec(S)) + ":" + y.lastIndex);

// --- String methods: /g starts from 0 and ends at 0; plain leaves lastIndex alone ---
const rg = /a/g;
rg.lastIndex = 3;
console.log("replace-g=" + "aaa".replace(rg, "-") + ":" + rg.lastIndex);
const rp = /a/;
rp.lastIndex = 3;
console.log("replace-plain=" + "aaa".replace(rp, "-") + ":" + rp.lastIndex);
const rgy = /a/gy;
rgy.lastIndex = 1;
console.log("replace-gy=" + "aab".replace(rgy, "-") + ":" + rgy.lastIndex);
const mg = /a/g;
mg.lastIndex = 3;
console.log("match-g=" + (("aaa".match(mg) as any) || []).join(",") + ":" + mg.lastIndex);
const mp = /a/;
mp.lastIndex = 3;
console.log("match-plain=" + ("aaa".match(mp) as any).index + ":" + mp.lastIndex);
const s1 = /a/g;
s1.lastIndex = 9;
console.log("search-g=" + "xa".search(s1) + ":" + s1.lastIndex);
const s2 = /a/y;
s2.lastIndex = 9;
console.log("search-y=" + "xa".search(s2) + ":" + s2.lastIndex);
const sp = /,/g;
sp.lastIndex = 5;
console.log("split-g=" + "a,b,c".split(sp).join("|") + ":" + sp.lastIndex);

// --- Symbol methods called directly follow the same write-back rules ---
const sg = /a/g;
sg.lastIndex = 2;
console.log("sym-replace-g=" + sg[Symbol.replace]("aaa", "-") + ":" + sg.lastIndex);
const plainSym = /a/;
plainSym.lastIndex = 2;
console.log("sym-replace-plain=" + plainSym[Symbol.replace]("aaa", "-") + ":" + plainSym.lastIndex);
const stickySym = /a/y;
stickySym.lastIndex = 1;
console.log("sym-replace-sticky=" + stickySym[Symbol.replace]("baa", "-") + ":" + stickySym.lastIndex);
const stickyMiss = /a/y;
stickyMiss.lastIndex = 0;
console.log("sym-replace-sticky-miss=" + stickyMiss[Symbol.replace]("baa", "-") + ":" + stickyMiss.lastIndex);
const sgs = /c/g;
sgs.lastIndex = 9;
console.log("sym-search-g=" + sgs[Symbol.search]("abc") + ":" + sgs.lastIndex);
const sys = /a/y;
sys.lastIndex = 5;
console.log("sym-search-y=" + sys[Symbol.search]("xa") + ":" + sys.lastIndex);
const mgs = /a/g;
mgs.lastIndex = 2;
console.log("sym-match-g=" + (mgs[Symbol.match]("aaa") as any).length + ":" + mgs.lastIndex);
const spg = /,/g;
spg.lastIndex = 4;
console.log("sym-split=" + spg[Symbol.split]("a,b").join("|") + ":" + spg.lastIndex);

// --- the property is an own, writable, non-enumerable, non-configurable data slot ---
console.log("desc-literal=" + desc(/a/, "lastIndex"));
console.log("desc-global=" + desc(/a/g, "lastIndex"));
console.log("desc-ctor=" + desc(new RegExp("a", "y"), "lastIndex"));
console.log("own-names=" + Object.getOwnPropertyNames(/a/g).join(","));
console.log("keys=" + JSON.stringify(Object.keys(/a/g)));
console.log("on-proto=" + Object.prototype.hasOwnProperty.call(RegExp.prototype, "lastIndex"));

// --- writes accept any value; reads coerce it inside exec/test ---
const w = /a/g;
console.log("set-num=" + Reflect.set(w, "lastIndex", 3) + "/" + w.lastIndex);
console.log("set-str=" + Reflect.set(w, "lastIndex", "1") + "/" + JSON.stringify(w.lastIndex));
console.log("set-obj=" + Reflect.set(w, "lastIndex", {}) + "/" + typeof w.lastIndex);
console.log("set-neg=" + Reflect.set(w, "lastIndex", -5) + "/" + w.lastIndex);
w.lastIndex = 0;

const r1 = /a/g;
r1.lastIndex = "1" as any;
console.log("str-index-exec=" + JSON.stringify(r1.exec("aaa")) + "/" + r1.lastIndex + "/" + typeof r1.lastIndex);
const r2 = /a/g;
r2.lastIndex = -1 as any;
console.log("neg-index-exec=" + JSON.stringify(r2.exec("aaa")) + "/" + r2.lastIndex);
const r3 = /a/g;
r3.lastIndex = 1.7 as any;
console.log("frac-test=" + r3.test("xa") + ":" + r3.lastIndex);
const r4 = /a/g;
r4.lastIndex = NaN as any;
console.log("nan-test=" + r4.test("aaa") + ":" + r4.lastIndex);
const r5 = /a/g;
r5.lastIndex = 2.7 as any;
console.log("frac-exec=" + JSON.stringify(r5.exec("aaa")) + "/" + r5.lastIndex);

// --- exec reads lastIndex BEFORE it coerces the subject, so the coercion cannot move it ---
const order = /a/g;
order.lastIndex = 0;
const mover: any = { toString: () => { order.lastIndex = 1; return "aa"; } };
const ro: any = order.exec(mover);
console.log("order-index=" + ro.index + " after=" + order.lastIndex);

// --- non-configurable: cannot be deleted, re-typed, or made enumerable ---
const c = /a/g;
console.log("delete=" + Reflect.deleteProperty(c, "lastIndex"));
console.log("to-accessor=" + Reflect.defineProperty(c, "lastIndex", { get: () => 0 }));
console.log("to-enumerable=" + Reflect.defineProperty(c, "lastIndex", { enumerable: true }));
console.log("to-configurable=" + Reflect.defineProperty(c, "lastIndex", { configurable: true }));
console.log("after-attempts=" + desc(c, "lastIndex"));

// --- writable can be turned off once; afterwards stateful matching throws ---
const frozen = /a/g;
frozen.lastIndex = 0;
console.log("make-readonly=" + Reflect.defineProperty(frozen, "lastIndex", { writable: false }));
console.log("readonly-desc=" + desc(frozen, "lastIndex"));
console.log("re-enable=" + Reflect.defineProperty(frozen, "lastIndex", { writable: true }));
console.log("ro-set=" + Reflect.set(frozen, "lastIndex", 2) + "/" + frozen.lastIndex);
console.log("ro-make-again=" + Reflect.defineProperty(frozen, "lastIndex", { writable: false }));
console.log("ro-exec=" + attempt(() => frozen.exec("a")));
console.log("ro-test=" + attempt(() => frozen.test("a")));
console.log("ro-replace=" + attempt(() => "a".replace(frozen, "-")));
console.log("ro-match=" + attempt(() => "a".match(frozen)));

const stickyRo = /a/y;
Reflect.defineProperty(stickyRo, "lastIndex", { writable: false });
console.log("ro-y-exec=" + attempt(() => stickyRo.exec("a")));

// a failing /g match must still write 0, so it throws too
const failRo = /zz/g;
Reflect.defineProperty(failRo, "lastIndex", { writable: false });
console.log("ro-g-fail=" + attempt(() => failRo.exec("a")));

// a PLAIN regex never writes lastIndex, so a read-only one keeps working
const plainRo = /a/;
Reflect.defineProperty(plainRo, "lastIndex", { writable: false });
console.log("ro-plain-exec=" + attempt(() => JSON.stringify(plainRo.exec("xa"))));
console.log("ro-plain-test=" + attempt(() => plainRo.test("xa")));
console.log("ro-plain-lastIndex=" + plainRo.lastIndex);

// Object.freeze on the whole regex implies the same
const whole = /a/g;
Object.freeze(whole);
console.log("frozen-obj=" + Object.isFrozen(whole));
console.log("frozen-obj-exec=" + attempt(() => whole.exec("a")));
console.log("frozen-obj-plain=" + attempt(() => JSON.stringify(Object.freeze(/a/).exec("a"))));
