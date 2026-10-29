// yield as an expression and as a statement: operands evaluated around a yield in
// source order, value-yield inside if / try / switch / labeled blocks / loops,
// break and continue inside generator loops, and generator EXPRESSIONS that
// capture and mutate variables of the enclosing scope (including ++/--).

// Drives a generator, sending sends[i] on the i-th resume, and renders every
// yielded value and the final return value.
function drive(g: Generator<any, any, any>, sends: any[]): string {
  const out: string[] = [];
  let r = g.next();
  let i = 0;
  while (!r.done) {
    out.push("y:" + String(r.value));
    r = g.next(sends[i++]);
  }
  out.push("ret:" + String(r.value));
  return out.join(" ");
}

// Collects every value of an iterable, bracketed, so empty and nested cases read clearly.
function collect(it: Iterable<unknown>): string {
  let o = "";
  for (const v of it) o = o + "[" + v + "]";
  return o;
}

const trace: string[] = [];
function side(tag: string, v: any) {
  trace.push(tag);
  return v;
}

// ── yield in expression positions: evaluation order around the suspension ───
function* binary(): Generator<any, any, any> {
  const sum = (yield "L") + (yield "R");
  return sum;
}
console.log("binary=" + drive(binary(), [10, 5]));

function* inArray(): Generator<any, any, any> {
  const arr = [yield "a", side("mid", 99), yield "b"];
  return arr.join("|");
}
trace.length = 0;
console.log("array=" + drive(inArray(), ["A", "B"]) + " sides=" + trace.join(","));

function join3(a: any, b: any, c: any) {
  trace.push("callee-run");
  return a + "/" + b + "/" + c;
}
function* inCall(): Generator<any, any, any> {
  return side("callee-read", join3)(yield "x", side("arg2", "M"), yield "y");
}
trace.length = 0;
console.log("call=" + drive(inCall(), ["X", "Y"]) + " order=" + trace.join(","));

function* inObject(): Generator<any, any, any> {
  const o: any = { [yield "key"]: yield "value", fixed: side("fixed", 1) };
  return JSON.stringify(o);
}
trace.length = 0;
console.log("object=" + drive(inObject(), ["k1", "v1"]) + " sides=" + trace.join(","));

function* memberAssign(): Generator<any, any, any> {
  const holder: any = { slot: "initial" };
  side("holder", holder)[yield "prop"] = yield "val";
  return JSON.stringify(holder);
}
trace.length = 0;
console.log("memberAssign=" + drive(memberAssign(), ["p", "V"]) + " sides=" + trace.join(","));

function* inTemplate(): Generator<any, any, any> {
  return `pre-${yield "t1"}-mid-${yield "t2"}-post`;
}
console.log("template=" + drive(inTemplate(), ["A", "B"]));

function* ternary(flag: boolean): Generator<any, any, any> {
  const v = flag ? yield "then" : yield "else";
  return String(v);
}
console.log("ternaryTrue=" + drive(ternary(true), ["T"]));
console.log("ternaryFalse=" + drive(ternary(false), ["F"]));

function* shortCircuit(): Generator<any, any, any> {
  const a = false && (yield "never1");
  const b = true || (yield "never2");
  const c = null ?? (yield "taken");
  return String(a) + "," + String(b) + "," + String(c);
}
console.log("shortCircuit=" + drive(shortCircuit(), ["C"]));

function* unary(): Generator<any, any, any> {
  const neg = -(yield "num");
  const cmp = (yield "left") < (yield "right");
  return neg + "," + cmp;
}
console.log("unary=" + drive(unary(), [7, 1, 2]));

// a compound assignment reads its target BEFORE the yield resumes
function* compound(): Generator<any, any, any> {
  let acc = 10;
  acc += yield "add";
  acc *= yield "mul";
  return String(acc);
}
console.log("compound=" + drive(compound(), [5, 2]));

function* inSwitch(): Generator<any, any, any> {
  switch (yield "which") {
    case "a":
      return "chose-a:" + (yield "a-extra");
    case "b":
      return "chose-b";
    default:
      return "chose-default";
  }
}
console.log("switchA=" + drive(inSwitch(), ["a", "EX"]));
console.log("switchB=" + drive(inSwitch(), ["b"]));
console.log("switchDefault=" + drive(inSwitch(), ["zzz"]));

function* inner(prefix: string) {
  yield prefix + "-i1";
  yield prefix + "-i2";
  return prefix + "-done";
}
function* delegating(): Generator<any, any, any> {
  const r = yield* inner(yield "prefix");
  return "delegated:" + r;
}
console.log("yieldInsideDelegate=" + drive(delegating(), ["P"]));

function* bare(): Generator<any, any, any> {
  const got = yield;
  return "got:" + String(got);
}
console.log("bare=" + drive(bare(), ["S"]));

// ── value-yield inside if, try, switch, labeled blocks ──────────────────────
function* withBlock() {
  if (true) {
    const v = yield 1;
    yield v;
  }
}
const ia = withBlock();
console.log("blockFirst=" + ia.next().value);
console.log("blockSent=" + ia.next(5).value);

function* thenBranch() {
  const v = yield 1;
  if (v > 0) yield v * 2;
  else yield 0;
}
const ib = thenBranch();
ib.next();
console.log("thenTaken=" + ib.next(5).value);

function* elseBranch() {
  const v = yield 1;
  if (v > 0) yield v * 2;
  else yield -1;
}
const ic = elseBranch();
ic.next();
console.log("elseTaken=" + ic.next(-3).value);

function* nestedIf() {
  const x = yield 1;
  if (x > 10) {
    const y = yield x;
    yield y + 1;
  } else {
    yield 0;
  }
}
const ig = nestedIf();
console.log("nestedIf=" + ig.next().value + "," + ig.next(20).value + "," + ig.next(7).value);

function* elseIfChain() {
  const v = yield 1;
  if (v === 1) yield "um";
  else if (v === 2) yield "dois";
  else yield "outro";
}
const ie = elseIfChain();
ie.next();
console.log("elseIfChain=" + ie.next(2).value);

function* falseBranch() {
  if (false) {
    yield 9;
  }
  yield 1;
}
console.log("branchNotTaken=" + [...falseBranch()].join(","));

function* noBraces() {
  if (true) yield 1;
  else yield 2;
  yield 3;
}
console.log("noBraces=" + [...noBraces()].join(","));

function* withReturnInIf() {
  if (true) {
    return;
  }
  yield 1;
}
console.log("withReturnInIf=[" + [...withReturnInIf()].join(",") + "]");

function* tryCatchValue() {
  try {
    const v = yield 1;
    yield v;
  } catch (e) {}
}
const it1 = tryCatchValue();
console.log("emptyCatchFirst=" + it1.next().value);
console.log("emptyCatchSent=" + it1.next(5).value);

function* catchWithYield() {
  try {
    const v = yield 1;
    yield v;
  } catch (e) {
    yield 9;
  }
}
const it2 = catchWithYield();
it2.next();
console.log("catchYieldSent=" + it2.next(5).value);

function* continuesAfter() {
  try {
    const v = yield 1;
    yield v * 2;
  } catch (e) {}
  yield 99;
}
const it3 = continuesAfter();
console.log("afterTry=" + it3.next().value + "," + it3.next(3).value + "," + it3.next().value);

function* catchBodyNotRun() {
  let mark = 0;
  try {
    const v = yield 1;
    yield v;
  } catch (e) {
    mark = 1;
  }
  yield mark;
}
const it4 = catchBodyNotRun();
it4.next();
it4.next(7);
console.log("catchNotRun=" + it4.next().value);

function* tryFinallyPlain() {
  try {
    yield 1;
  } finally {}
  yield 2;
}
console.log("tryFinallyPlain=" + [...tryFinallyPlain()].join(","));

// yield inside try, switch and a labeled block, run through for-of
function coletaFabrica() {
  let falhou = false;
  const g = function* (x: string) {
    if (x !== "pula") {
      try {
        yield "t:" + x;
      } catch (e) {
        falhou = true;
      }
    }
    yield "fim";
  };
  return {
    g: g,
    ok: function (): boolean {
      return !falhou;
    },
  };
}
const m = coletaFabrica();
console.log("try_com=" + collect(m.g("a")));
console.log("try_sem=" + collect(m.g("pula")));
console.log("captura_intacta=" + m.ok());

let marca = "";
const labelGen = function* (n: number) {
  switch (n) {
    case 1:
      yield "um";
      break;
    default:
      yield "outro";
  }
  L: {
    marca = "visitou";
    yield "rot";
  }
};
console.log("switch_1=" + collect(labelGen(1)));
console.log("switch_d=" + collect(labelGen(9)));
console.log("rotulo_efeito=" + marca);

let passos = "";
const finGen = function* () {
  try {
    yield "corpo";
  } finally {
    passos = passos + "F";
  }
  yield "depois";
};
console.log("finally=" + collect(finGen()));
console.log("finally_rodou=" + passos);

// ── break and continue inside generator loops (eager fallback must still stop) ──
function* breakWhile() {
  let i = 0;
  while (true) {
    if (i >= 2) break;
    yield i;
    i = i + 1;
  }
}
console.log("breakWhile=" + [...breakWhile()].join(","));

function* breakFor() {
  for (let i = 0; i < 5; i++) {
    if (i === 2) break;
    yield i;
  }
}
console.log("breakFor=" + [...breakFor()].join(","));

function* continueForOf() {
  for (const x of [1, 2, 3]) {
    if (x === 2) continue;
    yield x;
  }
}
console.log("continueForOf=" + [...continueForOf()].join(","));

function* breakDoWhile() {
  let i = 0;
  do {
    if (i >= 2) break;
    yield i;
    i = i + 1;
  } while (true);
}
console.log("breakDoWhile=" + [...breakDoWhile()].join(","));

function* continueWhile() {
  let i = 0;
  while (i < 4) {
    i = i + 1;
    if (i % 2 === 0) continue;
    yield i;
  }
}
console.log("continueWhile=" + [...continueWhile()].join(","));

const bw = breakWhile();
console.log("breakNext=" + bw.next().value + "," + bw.next().value + "," + JSON.stringify(bw.next()));

// stays lazy: an infinite loop is only pulled as far as the consumer asks
function* infinite() {
  let i = 0;
  while (true) {
    yield i;
    i = i + 1;
  }
}
const inf = infinite();
console.log("infiniteLazy=" + inf.next().value + "," + inf.next().value);

// a value-yield loop that folds the sent values into its state
function* withSent() {
  let i = 0;
  while (i < 3) {
    const v = yield i;
    i = i + (v || 1);
  }
}
const ws = withSent();
console.log("sentValue=" + ws.next().value + "," + ws.next(2).value);

function* noBreak() {
  let i = 0;
  while (i < 2) {
    yield i;
    i = i + 1;
  }
}
console.log("noBreak=" + [...noBreak()].join(","));

// ── generator EXPRESSIONS capturing the enclosing scope ─────────────────────
function withCapture() {
  const o = { v: 5 };
  function r(x: number) {
    return x + 1;
  }
  const g = function* () {
    yield r(o.v);
  };
  return g().next().value;
}
console.log("capturedCall=" + withCapture());

function captureSpread() {
  const base = 10;
  const g = function* () {
    yield base;
    yield base + 1;
  };
  return [...g()].join(",");
}
console.log("captureSpread=" + captureSpread());

function captureForOf() {
  const arr = [1, 2, 3];
  const g = function* () {
    for (const x of arr) yield x * 2;
  };
  let s = 0;
  for (const v of g()) {
    s = s + v;
  }
  return s;
}
console.log("captureForOf=" + captureForOf());

function captureTwoLevels() {
  const a = 100;
  function middle() {
    const b = 20;
    const g = function* () {
      yield a + b;
    };
    return g().next().value;
  }
  return middle();
}
console.log("captureTwoLevels=" + captureTwoLevels());

// each call to the outer function captures its OWN scope
function makeCounter(start: number) {
  const g = function* () {
    yield start;
    yield start + 1;
  };
  return [...g()].join(",");
}
console.log("independentA=" + makeCounter(1));
console.log("independentB=" + makeCounter(50));

// value-yield inside a capturing generator expression: the sent value must land
// in the local, not be mistaken for a statement yield
function withObject() {
  const o = { v: 5 };
  const g = function* () {
    const a = yield o.v;
    yield a * 2;
  };
  const it = g();
  return it.next().value + "," + it.next(10).value;
}
console.log("captureValueObject=" + withObject());

function withFnAndConst() {
  const mult = 3;
  function h(x: number) {
    return x + 1;
  }
  const g = function* () {
    const a = yield h(1);
    yield a * mult;
  };
  const it = g();
  return it.next().value + "," + it.next(4).value;
}
console.log("captureValueFnAndConst=" + withFnAndConst());

function withParam(p: number) {
  const g = function* () {
    const a = yield p;
    yield a + p;
  };
  const it = g();
  return it.next().value + "," + it.next(10).value;
}
console.log("captureValueParam=" + withParam(2));

// a captured variable WRITTEN by the generator is shared with the outer scope
function contador() {
  let n = 0;
  const g = function* () {
    n = n + 1;
    const a = yield n;
    n = n + 10;
    const b = yield n;
    return a + "/" + b;
  };
  return {
    g: g,
    ler: function (): number {
      return n;
    },
  };
}
const c = contador();
console.log("passos=" + drive(c.g(), ["s", "t"]));
console.log("visto_de_fora=" + c.ler());

// the memoization idiom of transpiled async code: init runs exactly once
function fabrica() {
  let cache: any;
  let inits = 0;
  const g = function* (x: number) {
    const m = cache || (cache = (function () {
      inits = inits + 1;
      return { nome: "M" };
    })());
    return yield m.nome + "/" + x;
  };
  return {
    g: g,
    inits: function (): number {
      return inits;
    },
    cache: function (): any {
      return cache;
    },
  };
}
const fa = fabrica();
console.log("primeira=" + drive(fa.g(1), ["A"]));
console.log("segunda=" + drive(fa.g(2), ["B"]));
console.log("inits=" + fa.inits());
console.log("escapou=" + (fa.cache() !== undefined));

// ++ and -- on a captured variable: postfix yields the OLD value, prefix the NEW
function incdec() {
  let n = 0;
  let m = 10;
  const g = function* () {
    const a = yield "pos:" + n++;
    const b = yield "pre:" + ++n;
    const c2 = yield "dec_pos:" + m--;
    const d = yield "dec_pre:" + --m;
    return a + "/" + b + "/" + c2 + "/" + d;
  };
  return {
    g: g,
    n: function (): number {
      return n;
    },
    m: function (): number {
      return m;
    },
  };
}
const idc = incdec();
console.log("incdec=" + drive(idc.g(), ["S", "T", "U", "V"]));
console.log("incdecN=" + idc.n() + " incdecM=" + idc.m());

// two instances of the SAME generator share the captured variable
const idc2 = incdec();
drive(idc2.g(), ["A", "B", "C", "D"]);
drive(idc2.g(), ["A", "B", "C", "D"]);
console.log("accumulated=" + idc2.n());

// ++ on a captured variable inside try, with a return that reads it
function countTries() {
  let tentativas = 0;
  const g = function* (x: string) {
    try {
      tentativas++;
      const r = yield "t:" + x;
      return r;
    } catch (e) {
      return "erro";
    }
  };
  return {
    g: g,
    tentativas: function (): number {
      return tentativas;
    },
  };
}
const ct = countTries();
console.log("tryRun1=" + drive(ct.g("a"), ["X"]));
console.log("tryRun2=" + drive(ct.g("b"), ["Y"]));
console.log("tentativas=" + ct.tentativas());

console.log("end");
