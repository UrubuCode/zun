// Abrupt completions: finally overriding or cancelling try/catch outcomes,
// throw replacement, for-of iterator closing, and catch-parameter binding.

const trace: string[] = [];
function tag(e: any): string {
  return e && typeof e === "object" && "tag" in e ? String(e.tag) : String(e);
}

// --- finally that completes abruptly cancels whatever was in flight ---
function breakSwallows(): string {
  const seen: string[] = [];
  for (let i = 0; i < 3; i++) {
    try {
      seen.push("try" + i);
      throw { tag: "boom" + i };
    } finally {
      seen.push("fin" + i);
      break;
    }
  }
  seen.push("after");
  return seen.join(",");
}
console.log("break_swallows=" + breakSwallows());

function continueSwallows(): string {
  const seen: string[] = [];
  for (let i = 0; i < 3; i++) {
    try {
      throw { tag: "x" + i };
    } finally {
      seen.push("fin" + i);
      continue;
    }
  }
  seen.push("loop-ended-normally");
  return seen.join(",");
}
console.log("continue_swallows=" + continueSwallows());

function returnSwallows(): string {
  try {
    throw { tag: "never-seen" };
  } finally {
    return "from-finally";
  }
}
console.log("return_swallows=" + returnSwallows());

const evaluated: string[] = [];
function returnReplacesReturn(): string {
  try {
    evaluated.push("try-expr");
    return "try-value";
  } finally {
    evaluated.push("finally-expr");
    return "finally-value";
  }
}
console.log("return_replaces=" + returnReplacesReturn() + "|" + evaluated.join(","));

function labelledBreak(): string {
  const seen: string[] = [];
  outer: for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 3; j++) {
      try {
        if (i === 1 && j === 1) throw { tag: "deep" };
        seen.push(i + "" + j);
      } finally {
        if (i === 1 && j === 1) {
          seen.push("fin");
          break outer;
        }
      }
    }
  }
  seen.push("end");
  return seen.join(",");
}
console.log("labelled_break=" + labelledBreak());

function labelledBlockBreak(): string {
  const seen: string[] = [];
  block: {
    try {
      seen.push("in-block");
      throw { tag: "blocked" };
    } finally {
      seen.push("fin");
      break block;
    }
  }
  seen.push("after-block");
  return seen.join(",");
}
console.log("labelled_block_break=" + labelledBlockBreak());

function onlyTheAbruptOneCancels(): string {
  const seen: string[] = [];
  for (let i = 0; i < 2; i++) {
    try {
      try {
        throw { tag: "inner" + i };
      } finally {
        seen.push("quiet" + i);
      }
    } finally {
      seen.push("abrupt" + i);
      continue;
    }
  }
  seen.push("survived");
  return seen.join(",");
}
console.log("only_abrupt_cancels=" + onlyTheAbruptOneCancels());

function breakOverReturn(): string {
  for (let i = 0; i < 3; i++) {
    try {
      return "early" + i;
    } finally {
      break;
    }
  }
  return "fell-through";
}
console.log("break_over_return=" + breakOverReturn());

function continueOverReturn(): string {
  const seen: string[] = [];
  for (let i = 0; i < 3; i++) {
    try {
      seen.push("want-return" + i);
      return "never";
    } finally {
      continue;
    }
  }
  return seen.join(",") + "|fell-through";
}
console.log("continue_over_return=" + continueOverReturn());

function outerCatchSeesNothing(): string {
  let caught = "none";
  try {
    for (let i = 0; i < 1; i++) {
      try {
        throw { tag: "vanishes" };
      } finally {
        break;
      }
    }
  } catch (e) {
    caught = tag(e);
  }
  return caught;
}
console.log("outer_catch_sees=" + outerCatchSeesNothing());

function whileContinue(): string {
  const seen: string[] = [];
  let n = 0;
  while (n < 3) {
    try {
      n++;
      throw { tag: "w" + n };
    } finally {
      seen.push("f" + n);
      continue;
    }
  }
  return seen.join(",") + "|n=" + n;
}
console.log("while_continue=" + whileContinue());

function doWhileContinue(): string {
  const seen: string[] = [];
  let n = 0;
  do {
    try {
      seen.push("body" + n);
      throw { tag: "d" };
    } finally {
      n++;
      continue;
    }
  } while (n < 3);
  return seen.join(",") + "|n=" + n;
}
console.log("do_while_continue=" + doWhileContinue());

function forOfBreak(): string {
  const seen: string[] = [];
  for (const v of ["a", "b", "c"]) {
    try {
      seen.push("saw:" + v);
      if (v === "b") throw { tag: "at-b" };
    } finally {
      if (v === "b") {
        seen.push("fin-b");
        break;
      }
    }
  }
  seen.push("done");
  return seen.join(",");
}
console.log("for_of_break=" + forOfBreak());

function returnCancelsCatchThrow(): string {
  try {
    throw { tag: "one" };
  } catch (e) {
    throw { tag: "two" };
  } finally {
    return "finally-return-wins";
  }
}
console.log("return_cancels_catch=" + returnCancelsCatchThrow());

function innerBreakOuterQuiet(): string {
  const seen: string[] = [];
  try {
    for (let i = 0; i < 3; i++) {
      try {
        throw { tag: "i" + i };
      } finally {
        seen.push("inner" + i);
        break;
      }
    }
    seen.push("after-loop");
  } finally {
    seen.push("outer-finally");
  }
  return seen.join(",");
}
console.log("inner_break_outer_quiet=" + innerBreakOuterQuiet());

function finallyReturnSeesMutation(): string {
  let state = "initial";
  try {
    state = "mutated";
    throw { tag: "ignored" };
  } finally {
    return "state=" + state;
  }
}
console.log("finally_return_sees=" + finallyReturnSeesMutation());

function returnOverBreak(): string {
  for (let i = 0; i < 3; i++) {
    try {
      break;
    } finally {
      return "returned-instead-of-breaking";
    }
  }
  return "loop-ended";
}
console.log("return_over_break=" + returnOverBreak());

function nothingPending(): string {
  const seen: string[] = [];
  for (let i = 0; i < 3; i++) {
    try {
      seen.push("i" + i);
    } finally {
      if (i === 1) break;
    }
  }
  return seen.join(",");
}
console.log("nothing_pending=" + nothingPending());

function labelledContinue(): string {
  const seen: string[] = [];
  outer: for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 3; j++) {
      try {
        if (j === 1) throw { tag: "skip" };
        seen.push(i + "" + j);
      } finally {
        if (j === 1) continue outer;
      }
    }
    seen.push("inner-done" + i);
  }
  return seen.join(",");
}
console.log("labelled_continue=" + labelledContinue());

function forInContinue(): string {
  const seen: string[] = [];
  const src: any = { p: 1, q: 2 };
  for (const k in src) {
    try {
      throw { tag: k };
    } finally {
      seen.push("k:" + k);
      continue;
    }
  }
  return seen.join(",") + "|survived";
}
console.log("for_in_continue=" + forInContinue());

function catchReturnedThenThrow(): string {
  const seen: string[] = [];
  function inner(): string {
    try {
      throw { tag: "start" };
    } catch (e) {
      seen.push("catch-returns");
      return "catch-value";
    } finally {
      seen.push("finally-throws");
      throw { tag: "beats-return" };
    }
  }
  try {
    seen.push("got:" + inner());
  } catch (e) {
    seen.push("caught:" + tag(e));
  }
  return seen.join(",");
}
console.log("catch_returned_then_throw=" + catchReturnedThenThrow());

// --- a throw in finally replaces the completion in flight, and the outermost one escapes ---
function replaceThrow(): void {
  try {
    trace.push("try");
    throw { tag: "from-try" };
  } finally {
    trace.push("finally");
    throw { tag: "from-finally" };
  }
}
try {
  replaceThrow();
} catch (e) {
  console.log("replace_throw=" + tag(e) + "|" + trace.join(","));
}
function replaceReturn(): string {
  try {
    return "returned";
  } finally {
    throw { tag: "over-return" };
  }
}
try {
  replaceReturn();
} catch (e) {
  console.log("replace_return=" + tag(e));
}
trace.length = 0;
function replaceCatchThrow(): void {
  try {
    throw { tag: "a" };
  } catch (e) {
    trace.push("catch:" + tag(e));
    throw { tag: "from-catch" };
  } finally {
    trace.push("finally2");
    throw { tag: "wins" };
  }
}
try {
  replaceCatchThrow();
} catch (e) {
  console.log("replace_catch=" + tag(e) + "|" + trace.join(","));
}
trace.length = 0;
function nestedBoth(): void {
  try {
    try {
      throw { tag: "innermost" };
    } finally {
      trace.push("inner-finally");
      throw { tag: "inner-replacement" };
    }
  } finally {
    trace.push("outer-finally");
    throw { tag: "outer-replacement" };
  }
}
try {
  nestedBoth();
} catch (e) {
  console.log("nested_both=" + tag(e) + "|" + trace.join(","));
}
function threeLevels(): void {
  try {
    try {
      try {
        throw { tag: "L0" };
      } finally {
        throw { tag: "L1" };
      }
    } finally {
      throw { tag: "L2" };
    }
  } finally {
    throw { tag: "L3" };
  }
}
try {
  threeLevels();
} catch (e) {
  console.log("three_levels=" + tag(e));
}
function innerThrowOuterCatch(): string {
  try {
    try {
      throw { tag: "original" };
    } finally {
      throw { tag: "substituted" };
    }
  } catch (e) {
    return tag(e);
  }
}
console.log("inner_throw_outer_catch=" + innerThrowOuterCatch());
function breakThenThrow(): string {
  const seen: string[] = [];
  try {
    for (let i = 0; i < 3; i++) {
      try {
        seen.push("i" + i);
        if (i === 1) break;
      } finally {
        seen.push("f" + i);
        if (i === 1) throw { tag: "over-break" };
      }
    }
    seen.push("after-loop");
  } catch (e) {
    seen.push("caught:" + tag(e));
  }
  return seen.join(",");
}
console.log("break_then_throw=" + breakThenThrow());
function passthrough(): void {
  try {
    throw { tag: "kept" };
  } finally {
    trace.push("quiet-finally");
  }
}
trace.length = 0;
try {
  passthrough();
} catch (e) {
  console.log("passthrough=" + tag(e) + "|" + trace.join(","));
}
function throwFromNothing(): string {
  try {
    return "plain";
  } finally {
    throw { tag: "unprovoked" };
  }
}
try {
  throwFromNothing();
} catch (e) {
  console.log("from_nothing=" + tag(e));
}
try {
  throw new TypeError("original");
} catch (e) {
  try {
    throw new RangeError("replacement");
  } catch (inner) {
    console.log("native_nested=" + (inner as any).constructor.name + "|" + (e as any).constructor.name);
  }
}
function nativeReplaced(): void {
  try {
    (null as any).boom;
  } finally {
    throw new RangeError("replacement");
  }
}
try {
  nativeReplaced();
} catch (e) {
  console.log("native_replaced=" + (e as any).constructor.name + "|range=" + (e instanceof RangeError) + "|type=" + (e instanceof TypeError));
}
function loopStopsAtReplacement(): string {
  const seen: string[] = [];
  try {
    for (let i = 0; i < 4; i++) {
      try {
        seen.push("i" + i);
        if (i === 1) throw { tag: "inner" + i };
      } finally {
        seen.push("f" + i);
        if (i === 1) throw { tag: "replacement" };
      }
    }
  } catch (e) {
    seen.push("caught:" + tag(e));
  }
  return seen.join(",");
}
console.log("loop_stops=" + loopStopsAtReplacement());
function siblingBlocks(): string {
  const seen: string[] = [];
  try {
    try {
      seen.push("first-try");
    } finally {
      seen.push("first-finally");
      throw { tag: "from-first" };
    }
  } catch (e) {
    seen.push("caught:" + tag(e));
  }
  try {
    seen.push("second-try");
  } finally {
    seen.push("second-finally");
  }
  return seen.join(",");
}
console.log("sibling_blocks=" + siblingBlocks());
const literalMarker = { tag: "literal" };
function identityKept(): string {
  try {
    try {
      throw new TypeError("original");
    } finally {
      throw literalMarker;
    }
  } catch (e) {
    return String(e === literalMarker) + ":" + (e as any).constructor.name;
  }
}
console.log("identity_kept=" + identityKept());

// --- iterator closing: break, return, labelled break and throw call return() once ---
const ilog: string[] = [];
function makeIterable(name: string, values: number[], opts?: any): any {
  const o = opts || {};
  return {
    [Symbol.iterator]() {
      let i = 0;
      return {
        next() {
          ilog.push(name + ".next" + i);
          if (o.throwOnNext === i) throw new RangeError("next failed");
          return i < values.length ? { value: values[i++], done: false } : { value: undefined, done: true };
        },
        return(v?: any) {
          ilog.push(name + ".return");
          if (o.throwOnReturn) throw new TypeError("return failed");
          return { value: v, done: true };
        },
      };
    },
  };
}
ilog.length = 0;
const exhausted: number[] = [];
for (const v of makeIterable("full", [1, 2])) exhausted.push(v);
console.log("exhausted=" + exhausted.join(",") + "|" + ilog.join(" "));
ilog.length = 0;
const upToTwo: number[] = [];
for (const v of makeIterable("brk", [1, 2, 3, 4])) {
  upToTwo.push(v);
  if (v === 2) break;
}
console.log("break_closes=" + upToTwo.join(",") + "|" + ilog.join(" "));
function returnsEarly(): string {
  for (const v of makeIterable("ret", [1, 2, 3])) {
    if (v === 2) return "left-at-" + v;
  }
  return "finished";
}
ilog.length = 0;
console.log("return_closes=" + returnsEarly() + "|" + ilog.join(" "));
ilog.length = 0;
outerIt: for (const a of makeIterable("A", [1, 2])) {
  for (const b of makeIterable("B", [10, 20])) {
    if (a === 1 && b === 20) break outerIt;
  }
}
console.log("labelled_closes=" + ilog.join(" "));
ilog.length = 0;
const kept: number[] = [];
for (const v of makeIterable("cont", [1, 2, 3])) {
  if (v === 2) continue;
  kept.push(v);
}
console.log("continue_no_close=" + kept.join(",") + "|" + ilog.join(" "));
ilog.length = 0;
const noReturn: any = {
  [Symbol.iterator]() {
    let i = 0;
    return { next() { ilog.push("nr.next"); return { value: i++, done: i > 5 }; } };
  },
};
const nrSeen: number[] = [];
for (const v of noReturn) {
  nrSeen.push(v);
  if (v === 1) break;
}
console.log("no_return_method=" + nrSeen.join(",") + "|" + ilog.join(" "));

const genLog: string[] = [];
function* counted(): Generator<number> {
  try {
    genLog.push("start");
    yield 1;
    genLog.push("between");
    yield 2;
    genLog.push("end");
  } finally {
    genLog.push("finally");
  }
}
const genSeen: number[] = [];
for (const v of counted()) {
  genSeen.push(v);
  if (v === 1) break;
}
console.log("gen_break=" + genSeen.join(",") + "|" + genLog.join(","));
genLog.length = 0;
const genAll: number[] = [];
for (const v of counted()) genAll.push(v);
console.log("gen_full=" + genAll.join(",") + "|" + genLog.join(","));
const gBroken = counted();
for (const v of gBroken) {
  if (v === 1) break;
}
const gAfter = gBroken.next();
console.log("gen_after_break=" + gAfter.done + "/" + String(gAfter.value));
function hasReturn(it: any): string { return typeof it.return; }
console.log("builtin_return=" + hasReturn([1][Symbol.iterator]()) + "|" + hasReturn(new Map([[1, 1]])[Symbol.iterator]()) + "|" + hasReturn("ab"[Symbol.iterator]()) + "|" + hasReturn(counted()));
const shared = [1, 2, 3, 4][Symbol.iterator]();
const firstPass: number[] = [];
for (const v of shared as any) {
  firstPass.push(v);
  if (v === 2) break;
}
const secondPass: number[] = [];
for (const v of shared as any) secondPass.push(v);
console.log("shared_iterator=" + firstPass.join(",") + "|" + secondPass.join(","));
ilog.length = 0;
const oddReturn: any = {
  [Symbol.iterator]() {
    let i = 0;
    return {
      next() { return { value: i++, done: false }; },
      return() { ilog.push("odd.return"); return { done: true }; },
    };
  },
};
for (const v of oddReturn) {
  if (v === 1) break;
}
console.log("odd_return=" + ilog.join(" "));

// --- iterator closing when the body or the loop head throws ---
ilog.length = 0;
function bodyThrows(): string {
  try {
    for (const v of makeIterable("body", [1, 2, 3])) {
      if (v === 2) throw new RangeError("from body");
    }
    return "no-throw";
  } catch (e) {
    return "caught:" + (e as any).constructor.name;
  }
}
console.log("body_throws=" + bodyThrows() + "|" + ilog.join(" "));
ilog.length = 0;
function bothThrow(): string {
  try {
    for (const v of makeIterable("both", [1, 2, 3], { throwOnReturn: true })) {
      if (v === 1) throw new RangeError("body wins");
    }
    return "no-throw";
  } catch (e) {
    return "caught:" + (e as any).constructor.name;
  }
}
console.log("body_beats_return=" + bothThrow() + "|" + ilog.join(" "));
ilog.length = 0;
function breakWithThrowingReturn(): string {
  try {
    for (const v of makeIterable("brk", [1, 2, 3], { throwOnReturn: true })) {
      if (v === 1) break;
    }
    return "no-throw";
  } catch (e) {
    return "caught:" + (e as any).constructor.name;
  }
}
console.log("break_throwing_return=" + breakWithThrowingReturn() + "|" + ilog.join(" "));
ilog.length = 0;
function nextThrows(): string {
  try {
    for (const v of makeIterable("nx", [1, 2, 3], { throwOnNext: 1 })) {
      ilog.push("body:" + v);
    }
    return "no-throw";
  } catch (e) {
    return "caught:" + (e as any).constructor.name;
  }
}
console.log("next_throws=" + nextThrows() + "|" + ilog.join(" "));
const gg: string[] = [];
function* guarded(): Generator<number> {
  try {
    yield 1;
    yield 2;
  } finally {
    gg.push("gen-finally");
  }
}
function genBodyThrows(): string {
  try {
    for (const v of guarded()) {
      gg.push("body" + v);
      if (v === 1) throw new RangeError("stop");
    }
    return "no-throw";
  } catch (e) {
    return "caught:" + (e as any).constructor.name;
  }
}
console.log("gen_body_throws=" + genBodyThrows() + "|" + gg.join(","));
function* badFinally(): Generator<number> {
  try {
    yield 1;
  } finally {
    throw new TypeError("finally failed");
  }
}
function bodyVsFinally(): string {
  try {
    for (const v of badFinally()) {
      if (v === 1) throw new RangeError("body");
    }
    return "no-throw";
  } catch (e) {
    return "caught:" + (e as any).constructor.name;
  }
}
console.log("body_vs_finally=" + bodyVsFinally());
function breakVsFinally(): string {
  try {
    for (const v of badFinally()) {
      if (v === 1) break;
    }
    return "no-throw";
  } catch (e) {
    return "caught:" + (e as any).constructor.name;
  }
}
console.log("break_vs_finally=" + breakVsFinally());
ilog.length = 0;
function patternThrows(): string {
  const rows: any = {
    [Symbol.iterator]() {
      let i = 0;
      const items = [
        { get v(): number { return 1; } },
        { get v(): number { throw new RangeError("bad getter"); } },
      ];
      return {
        next() {
          ilog.push("pat.next");
          return i < items.length ? { value: items[i++], done: false } : { done: true, value: undefined };
        },
        return() { ilog.push("pat.return"); return { done: true }; },
      };
    },
  };
  try {
    for (const { v } of rows) ilog.push("body:" + v);
    return "no-throw";
  } catch (e) {
    return "caught:" + (e as any).constructor.name;
  }
}
console.log("pattern_throws=" + patternThrows() + "|" + ilog.join(" "));
ilog.length = 0;
function nestedClose(): string {
  try {
    for (const a of makeIterable("O", [1, 2])) {
      for (const b of makeIterable("I", [10, 20])) {
        throw new RangeError("deep " + a + b);
      }
    }
    return "no-throw";
  } catch (e) {
    return "caught:" + (e as any).message;
  }
}
console.log("nested_close=" + nestedClose() + "|" + ilog.join(" "));
function badIterable(): string {
  try {
    for (const v of { [Symbol.iterator]: 5 } as any) {
      return "reached:" + v;
    }
    return "loop-empty";
  } catch (e) {
    return "caught:" + (e as any).constructor.name;
  }
}
console.log("bad_iterable=" + badIterable());
function notIterable(): string {
  let entered = false;
  try {
    for (const v of 42 as any) entered = true;
    return "loop-empty";
  } catch (e) {
    return "caught:" + (e as any).constructor.name + "|entered=" + entered;
  }
}
console.log("not_iterable=" + notIterable());

// --- catch parameters: a block-scoped binding that may be a pattern ---
let err = "outer-value";
try {
  throw "thrown-value";
} catch (err) {
  console.log("catch_inside=" + err);
  err = "reassigned-inside";
  console.log("catch_assigned=" + err);
}
console.log("catch_outside=" + err);
try {
  throw "param";
} catch (e) {
  {
    let e = "block-local";
    console.log("catch_body_shadow=" + e);
  }
  console.log("catch_param_intact=" + e);
}
try {
  throw { code: 42 };
} catch ({ code, kind = "unknown" }: any) {
  console.log("catch_object_pattern=" + code + "/" + kind);
}
try {
  throw [1, 2, 3, 4];
} catch ([, second, ...others]: any) {
  console.log("catch_array_pattern=" + second + "|" + others.join("-"));
}
const KEY = "detail";
try {
  throw { [KEY]: { line: 7, col: 3 } };
} catch ({ [KEY]: { line, col } }: any) {
  console.log("catch_nested_key=" + line + ":" + col);
}
const defaultsRun: string[] = [];
function fallback(name: string): string {
  defaultsRun.push(name);
  return "fb-" + name;
}
try {
  throw { a: "present" };
} catch ({ a = fallback("a"), b = fallback("b") }: any) {
  console.log("catch_defaults=" + a + "|" + b + "|" + defaultsRun.join(","));
}
try {
  throw { base: 10 };
} catch ({ base, scaled = base * 3 }: any) {
  console.log("catch_default_prior=" + base + "/" + scaled);
}
try {
  throw "xyz";
} catch ([first, ...tail]: any) {
  console.log("catch_string_pattern=" + first + "|" + tail.join(""));
}
try {
  throw Object.assign(new RangeError("ignored"), { where: "line-9" });
} catch (e) {
  const { where } = e as any;
  console.log("catch_error_field=" + where + "|" + (e as any).constructor.name);
}
const captured: Array<() => string> = [];
for (let i = 0; i < 3; i++) {
  try {
    throw "e" + i;
  } catch (caught) {
    captured.push(() => caught as string);
  }
}
console.log("catch_per_entry=" + captured.map((f) => f()).join(","));
function mutatedBinding(): string {
  try {
    throw "start";
  } catch (c) {
    const read = () => c;
    c = "mutated";
    return read() as string;
  }
}
console.log("catch_mutated=" + mutatedBinding());
function nestedCatch(): string {
  try {
    throw "outer-error";
  } catch (e) {
    let inner = "?";
    try {
      throw "inner-error";
    } catch (e) {
      inner = e as string;
    }
    return e + "|" + inner;
  }
}
console.log("catch_nested=" + nestedCatch());
function varEscapes(): string {
  try {
    throw "boom";
  } catch (e) {
    var leaked = "var-from-catch:" + e;
  }
  return leaked;
}
console.log("catch_var_escapes=" + varEscapes());
const kinds: string[] = [];
const thrown: any[] = [0, "", false, null, undefined, NaN];
for (const v of thrown) {
  try {
    throw v;
  } catch (e) {
    kinds.push(typeof e + ":" + String(e) + ":" + (e === v ? "same" : "changed"));
  }
}
console.log("catch_primitives=" + kinds.join(" "));
const marker = { id: "the-one" };
function rethrow(): string {
  try {
    try {
      throw marker;
    } catch (e) {
      throw e;
    }
  } catch (e) {
    return String(e === marker) + ":" + (e as any).id;
  }
}
console.log("rethrow_identity=" + rethrow());
function reassignedThenCalled(): string {
  try {
    throw "first";
  } catch (e) {
    const show = function (): string { return String(e); };
    const before = show();
    e = "second";
    return before + "->" + show();
  }
}
console.log("catch_reassigned=" + reassignedThenCalled());
function shadowsParam(e: string): string {
  const before = e;
  try {
    throw "inner";
  } catch (e) {
    return before + "|" + e;
  }
}
console.log("catch_shadows_param=" + shadowsParam("param-value"));
try {
  throw "scoped";
} catch (e) {
  console.log("catch_optional_binding_check=" + e);
} finally {
  console.log("finally_after_catch=ran");
}
let result = "none";
try {
  throw new Error("test");
} catch {
  result = "caught-without-binding";
}
console.log("optional_catch_binding=" + result);
