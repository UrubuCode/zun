"use strict";
// Throw and catch control flow: try/catch/finally completions, finally overriding
// returns and throws, rethrow across frames, labelled break and continue through
// finally, throws of non-Error values (identity preserved), errors crossing
// generator and async boundaries, iterator closing on abrupt exits, and errors
// raised from constructors. Only constructor names, messages the author wrote,
// booleans and ordered logs are printed.

function probe(fn: () => any): string {
  try {
    const v = fn();
    return "ok:" + String(v);
  } catch (e: any) {
    return e.constructor.name;
  }
}

// 1. Basic catch, finally, and the empty-message forms.
{
  try {
    throw new Error("boom");
  } catch (e: any) {
    console.log("err=" + e.name + ":" + e.message);
  } finally {
    console.log("fin=done");
  }
  try {
    throw new Error();
  } catch (e: any) {
    console.log("empty=" + e.name + ":[" + e.message + "]");
  }
  try {
    throw new TypeError();
  } catch (e: any) {
    console.log("empty-type=" + e.name + ":[" + e.message + "]");
  }
  try {
    throw "plain";
  } catch (e: any) {
    console.log("plain=" + e);
  }
}

// 2. Rethrow with a different class; instanceof chain on the new error.
{
  function nestedRethrow(): string {
    try {
      try {
        throw new TypeError("bad");
      } catch (e: any) {
        console.log("inner=" + e.name);
        throw new RangeError(e.message + ":range");
      }
    } catch (e: any) {
      return (
        e.name + ":" + e.message + ",error=" + (e instanceof Error) + ",type=" + (e instanceof TypeError) + ",range=" + (e instanceof RangeError)
      );
    }
  }
  console.log("outer=" + nestedRethrow());

  function parse(s: string): number {
    try {
      const n = JSON.parse(s);
      if (typeof n !== "number") throw new TypeError("not a number");
      return n;
    } catch (e) {
      if (e instanceof SyntaxError) throw new RangeError("invalid json: " + s);
      throw e;
    }
  }
  try {
    parse("{bad");
  } catch (e: any) {
    console.log("parse-syntax=" + e.name + ":" + e.message);
  }
  try {
    parse('"hello"');
  } catch (e: any) {
    console.log("parse-type=" + e.name + ":" + e.message);
  }
  console.log("parse-ok=" + parse("42"));
}

// 3. finally completions: an abrupt finally replaces the in-flight completion.
{
  function returnOverReturn(): string {
    try {
      return "try";
    } finally {
      return "finally";
    }
  }
  console.log("return-over-return=" + returnOverReturn());

  function returnOverThrow(): string {
    try {
      throw new RangeError("x");
    } finally {
      return "swallowed";
    }
  }
  console.log("return-over-throw=" + returnOverThrow());

  function throwOverThrow(): string {
    try {
      try {
        throw new RangeError("first");
      } finally {
        throw new TypeError("second");
      }
    } catch (e: any) {
      return e.constructor.name + ":" + e.message;
    }
  }
  console.log("throw-over-throw=" + throwOverThrow());

  function throwOverReturn(): string {
    try {
      (function (): string {
        try {
          return "value";
        } finally {
          throw new EvalError("late");
        }
      })();
      return "no-throw";
    } catch (e: any) {
      return e.constructor.name;
    }
  }
  console.log("throw-over-return=" + throwOverReturn());

  const log: string[] = [];
  function evaluatedFirst(): string {
    let v = "a";
    try {
      return v;
    } finally {
      v = "b";
      log.push("finally-ran");
    }
  }
  console.log("evaluated-first=" + evaluatedFirst() + ",log=" + log.join("|"));

  function normalCompletion(): string {
    const out: string[] = [];
    const r = (function (): string {
      try {
        out.push("try");
        return "kept";
      } finally {
        out.push("fin");
        "dropped";
      }
    })();
    return r + "/" + out.join(",");
  }
  console.log("normal=" + normalCompletion());

  function passthrough(): string {
    try {
      throw new URIError("kept");
    } catch (e: any) {
      return e.constructor.name;
    } finally {
      "ignored";
    }
  }
  console.log("passthrough=" + passthrough());

  function nested(): string {
    const out: string[] = [];
    try {
      try {
        try {
          throw new Error("deep");
        } finally {
          out.push("inner");
        }
      } finally {
        out.push("middle");
      }
    } catch (e: any) {
      out.push("caught:" + e.message);
    } finally {
      out.push("outer");
    }
    return out.join(",");
  }
  console.log("nested=" + nested());

  function rethrow(): string {
    const out: string[] = [];
    try {
      try {
        throw new TypeError("a");
      } catch (e: any) {
        out.push("catch:" + e.constructor.name);
        throw new RangeError("b");
      } finally {
        out.push("fin");
      }
    } catch (e: any) {
      out.push("outer:" + e.constructor.name);
    }
    return out.join("|");
  }
  console.log("rethrow=" + rethrow());

  function labelledBlock(): string {
    const out: string[] = [];
    block: {
      try {
        out.push("in");
        break block;
      } finally {
        out.push("fin");
      }
    }
    out.push("after");
    return out.join(",");
  }
  console.log("labelled=" + labelledBlock());

  const switchLog: string[] = [];
  function switchUnderFinally(v: number): string {
    try {
      switch (v) {
        case 1:
          return "one";
        default:
          return "other";
      }
    } finally {
      switchLog.push("switch-fin" + v);
    }
  }
  console.log("switch-1=" + switchUnderFinally(1) + ",switch-2=" + switchUnderFinally(2) + ",log=" + switchLog.join("|"));

  function breakOut(): string {
    const out: string[] = [];
    for (let i = 0; i < 3; i = i + 1) {
      try {
        out.push("body" + i);
        if (i === 1) break;
      } finally {
        out.push("fin" + i);
      }
    }
    return out.join(",");
  }
  console.log("break=" + breakOut());

  function breakInFinally(): string {
    const out: string[] = [];
    outer: for (let i = 0; i < 3; i = i + 1) {
      try {
        out.push("t" + i);
        continue outer;
      } finally {
        out.push("f" + i);
        if (i === 1) break outer;
      }
    }
    return out.join(",");
  }
  console.log("break-in-finally=" + breakInFinally());

  function continueInFinally(): string {
    const out: string[] = [];
    for (let i = 0; i < 3; i = i + 1) {
      try {
        if (i === 1) throw new Error("skip");
        out.push("ok" + i);
      } finally {
        if (i === 1) {
          out.push("swallow");
          continue;
        }
        out.push("fin" + i);
      }
    }
    return out.join(",");
  }
  console.log("continue-in-finally=" + continueInFinally());

  function loopFinally(): string {
    const out: string[] = [];
    for (const v of [1, 2, 3]) {
      try {
        if (v === 3) break;
        out.push("v" + v);
      } finally {
        out.push("f" + v);
      }
    }
    return out.join(",");
  }
  console.log("loop=" + loopFinally());
}

// 4. Catch-binding forms: optional, shadowing, destructuring.
{
  const shadow = "outer";
  function optionalBinding(): string {
    try {
      throw new Error("ignored");
    } catch {
      return "no-binding:" + shadow;
    }
  }
  console.log("optional=" + optionalBinding());

  function shadowing(): string {
    try {
      throw "thrown";
    } catch (shadow: any) {
      shadow = "reassigned";
      return "inner=" + shadow;
    }
  }
  console.log("shadowing=" + shadowing() + ",outer=" + shadow);

  function destructured(): string {
    try {
      throw { code: 7, detail: { tag: "t" } };
    } catch ({ code, detail: { tag } }: any) {
      return code + ":" + tag;
    }
  }
  console.log("destructured=" + destructured());

  function destructuredDefault(): string {
    try {
      throw {};
    } catch ({ code = "fallback" }: any) {
      return String(code);
    }
  }
  console.log("destructured-default=" + destructuredDefault());

  function destructureFails(): string {
    try {
      try {
        throw null;
      } catch ({ code }: any) {
        return "unreachable:" + String(code);
      }
    } catch (e: any) {
      return "secondary:" + e.constructor.name;
    }
  }
  console.log("destructure-null=" + destructureFails());

  function fresh(v: string): string {
    try {
      throw v;
    } catch (e: any) {
      return e;
    }
  }
  console.log("fresh=" + fresh("a") + "," + fresh("b"));
}

// 5. Non-Error throws: the catch binding receives the identical value.
{
  function classify(v: any): string {
    try {
      throw v;
    } catch (e: any) {
      const same = e === v || (typeof v === "number" && Number.isNaN(v) && Number.isNaN(e));
      return typeof e + "/" + String(same) + "/" + (e instanceof Error);
    }
  }
  console.log("string=" + classify("plain"));
  console.log("number=" + classify(42));
  console.log("zero=" + classify(0));
  console.log("nan=" + classify(NaN));
  console.log("boolean=" + classify(false));
  console.log("null=" + classify(null));
  console.log("undefined=" + classify(undefined));
  console.log("bigint=" + classify(10n));
  console.log("symbol=" + classify(Symbol("s")));
  console.log("object=" + classify({ a: 1 }));
  console.log("array=" + classify([1, 2]));
  console.log("function=" + classify(function named(): void {}));
  console.log("class=" + classify(class Thrown {}));
  console.log("frozen=" + classify(Object.freeze({ f: 1 })));
  console.log("error=" + classify(new TypeError("t")));

  const errorLike: any = { name: "TypeError", message: "looks-real", stack: "fake" };
  console.log("errorlike-instanceof=" + (errorLike instanceof Error));
  console.log("errorlike-tag=" + Object.prototype.toString.call(errorLike) + ",ctor=" + errorLike.constructor.name);
  console.log("errorlike-tostring=" + Error.prototype.toString.call(errorLike));

  function nameOf(v: any): string {
    try {
      throw v;
    } catch (e: any) {
      if (e === null || e === undefined) return "no-constructor";
      return e.constructor.name;
    }
  }
  console.log("name-string=" + nameOf("s") + ",number=" + nameOf(1) + ",bigint=" + nameOf(1n) + ",symbol=" + nameOf(Symbol("s")));
  console.log("name-null=" + nameOf(null) + ",undefined=" + nameOf(undefined) + ",array=" + nameOf([]));

  const token: any = { id: "token" };
  function level3(): void {
    throw token;
  }
  function level2(): void {
    try {
      level3();
    } catch (e) {
      throw e;
    }
  }
  function level1(): string {
    try {
      level2();
      return "never";
    } catch (e: any) {
      return String(e === token) + ":" + e.id;
    }
  }
  console.log("rethrow-identity=" + level1());

  const trace: string[] = [];
  function throughFinally(): string {
    try {
      try {
        throw token;
      } finally {
        trace.push("inner-finally");
      }
    } catch (e: any) {
      trace.push("caught");
      return String(e === token);
    } finally {
      trace.push("outer-finally");
    }
  }
  console.log("through-finally=" + throughFinally() + ",trace=" + trace.join(">"));
}

// 6. Errors from constructors: a throw inside the body, and a derived constructor
// that touches this before super().
{
  class Guarded extends Error {
    constructor(flag: boolean) {
      super("guarded");
      this.name = "Guarded";
      if (flag) {
        throw new RangeError("ctor-threw");
      }
    }
  }
  function construct(flag: boolean): string {
    try {
      const g = new Guarded(flag);
      return "built:" + g.name + ":" + g.message;
    } catch (e: any) {
      return "threw:" + e.constructor.name + ":" + e.message;
    }
  }
  console.log("ctor-ok=" + construct(false));
  console.log("ctor-throw=" + construct(true));

  class NoSuper extends Error {
    constructor() {
      (this as any).x = 1;
      super("m");
    }
  }
  console.log("this-before-super=" + probe(() => new NoSuper()));

  class FailingBase {
    constructor() {
      throw new EvalError("base");
    }
  }
  class FailingDerived extends FailingBase {
    constructor() {
      super();
    }
  }
  console.log("base-throws=" + probe(() => new FailingDerived()));
}

// 7. Generators: throw() resumes at the yield, return() runs finally, and a finally
// that returns or throws overrides the completion.
{
  const trace: string[] = [];
  function* guarded(): Generator<string, string, any> {
    try {
      yield "a";
      yield "b";
      return "normal-end";
    } catch (e: any) {
      yield "caught:" + e.constructor.name;
      return "after-catch";
    } finally {
      trace.push("finally");
    }
  }
  const g1 = guarded();
  console.log("g1-1=" + JSON.stringify(g1.next()));
  console.log("g1-throw=" + JSON.stringify(g1.throw(new RangeError("x"))));
  console.log("g1-2=" + JSON.stringify(g1.next()));
  console.log("g1-3=" + JSON.stringify(g1.next()));
  console.log("g1-trace=" + trace.join(","));

  trace.length = 0;
  const g2 = guarded();
  let early = "none";
  try {
    g2.throw(new TypeError("early"));
  } catch (e: any) {
    early = e.constructor.name;
  }
  console.log("g2-early=" + early + ",trace=" + trace.join(",") + ",after=" + JSON.stringify(g2.next()));

  trace.length = 0;
  const g3 = guarded();
  g3.next();
  console.log("g3-return=" + JSON.stringify(g3.return("early-exit")) + ",trace=" + trace.join(","));
  console.log("g3-after=" + JSON.stringify(g3.next()));

  function* stubborn(): Generator<string, string, any> {
    try {
      yield "work";
      return "unreached";
    } finally {
      yield "cleanup";
    }
  }
  const g4 = stubborn();
  console.log("g4-1=" + JSON.stringify(g4.next()));
  console.log("g4-return=" + JSON.stringify(g4.return("wanted")));
  console.log("g4-2=" + JSON.stringify(g4.next()));
  console.log("g4-3=" + JSON.stringify(g4.next()));

  function* overriding(): Generator<string, string, any> {
    try {
      yield "w";
      return "unreached";
    } finally {
      return "from-finally";
    }
  }
  const g5 = overriding();
  console.log("g5-1=" + JSON.stringify(g5.next()));
  console.log("g5-return=" + JSON.stringify(g5.return("wanted")));

  function* throwingFinally(): Generator<string, string, any> {
    try {
      yield "w";
      return "unreached";
    } finally {
      throw new EvalError("from-finally");
    }
  }
  const g6 = throwingFinally();
  g6.next();
  console.log("g6-return=" + probe(() => g6.return("wanted")));
  console.log("g6-after=" + JSON.stringify(g6.next()));

  const inner: string[] = [];
  function* innerGen(): Generator<string, string, any> {
    try {
      yield "i1";
      yield "i2";
      return "inner-done";
    } catch (e: any) {
      inner.push("inner-caught:" + e.constructor.name);
      return "inner-recovered";
    } finally {
      inner.push("inner-finally");
    }
  }
  function* outerGen(): Generator<string, string, any> {
    const v = yield* innerGen();
    yield "outer-saw:" + v;
    return "outer-done";
  }
  const g7 = outerGen();
  console.log("g7-1=" + JSON.stringify(g7.next()));
  console.log("g7-throw=" + JSON.stringify(g7.throw(new RangeError("d"))));
  console.log("g7-2=" + JSON.stringify(g7.next()));
  console.log("g7-inner=" + inner.join(","));

  inner.length = 0;
  const g8 = outerGen();
  g8.next();
  console.log("g8-return=" + JSON.stringify(g8.return("stop")) + ",inner=" + inner.join(","));

  function* explodes(): Generator<number> {
    yield 1;
    throw new URIError("boom");
  }
  const g9 = explodes();
  console.log("g9-1=" + JSON.stringify(g9.next()));
  console.log("g9-throw=" + probe(() => g9.next()));
  console.log("g9-done=" + JSON.stringify(g9.next()));
}

// 8. Async: rejections keep identity, finally overrides, await in finally.
const asyncLog: string[] = [];
{
  const marker = new RangeError("marker");

  async function throwsSync(): Promise<string> {
    throw marker;
  }
  async function throwsAfterAwait(): Promise<string> {
    await 0;
    throw marker;
  }
  async function finallyOverridesReturn(): Promise<string> {
    try {
      return "from-try";
    } finally {
      asyncLog.push("f1");
      return "from-finally";
    }
  }
  async function finallyOverridesThrow(): Promise<string> {
    try {
      throw new EvalError("inner");
    } finally {
      asyncLog.push("f2");
      return "rescued";
    }
  }
  async function finallyReplacesError(): Promise<string> {
    try {
      throw new EvalError("first");
    } finally {
      asyncLog.push("f3");
      throw new URIError("second");
    }
  }
  async function awaitInFinally(): Promise<string> {
    try {
      throw new EvalError("held");
    } finally {
      asyncLog.push("f4-before");
      await 0;
      asyncLog.push("f4-after");
    }
  }
  async function catchAndRethrow(): Promise<string> {
    try {
      await throwsAfterAwait();
      return "never";
    } catch (e: any) {
      asyncLog.push("caught:" + e.constructor.name + ":" + (e === marker));
      throw e;
    } finally {
      asyncLog.push("f5");
    }
  }
  async function settle(p: Promise<any>): Promise<string> {
    try {
      return "fulfilled:" + String(await p);
    } catch (e: any) {
      return "rejected:" + e.constructor.name;
    }
  }

  async function main(): Promise<void> {
    let same = "none";
    try {
      await throwsSync();
    } catch (e: any) {
      same = String(e === marker) + ":" + e.message;
    }
    console.log("identity=" + same);
    console.log("sync-throw=" + (await settle(throwsSync())));
    console.log("await-throw=" + (await settle(throwsAfterAwait())));
    console.log("finally-over-return=" + (await settle(finallyOverridesReturn())));
    console.log("finally-over-throw=" + (await settle(finallyOverridesThrow())));
    console.log("finally-replaces=" + (await settle(finallyReplacesError())));
    console.log("await-in-finally=" + (await settle(awaitInFinally())));
    console.log("rethrow=" + (await settle(catchAndRethrow())));
    console.log("async-log=" + asyncLog.join(">"));

    const p = throwsSync();
    console.log("is-promise=" + (p instanceof Promise) + ",settled=" + (await settle(p)));

    const badThenable: any = {
      then(): void {
        throw new SyntaxError("from-then");
      },
    };
    console.log("bad-thenable=" + (await settle(Promise.resolve().then(() => badThenable))));

    const rejecting: any = {
      then(_res: any, rej: any): void {
        rej(new URIError("thenable-reject"));
      },
    };
    console.log("rejecting-thenable=" + (await settle(Promise.resolve().then(() => rejecting))));

    const results = await Promise.allSettled([Promise.reject(new TypeError("a")), Promise.resolve("b"), throwsSync()]);
    console.log(
      "allsettled=" +
        results.map((r: any) => (r.status === "fulfilled" ? "f:" + r.value : "r:" + r.reason.constructor.name)).join("|"),
    );

    const failing: any = {
      [Symbol.asyncIterator]() {
        let i = 0;
        return {
          next(): Promise<any> {
            i++;
            if (i === 2) {
              return Promise.reject(new EvalError("iter"));
            }
            return Promise.resolve({ value: "v" + i, done: i > 3 });
          },
          return(): Promise<any> {
            asyncLog.push("async-return");
            return Promise.resolve({ done: true });
          },
        };
      },
    };
    const seen: string[] = [];
    let iterOutcome = "none";
    try {
      for await (const v of failing) {
        seen.push(v);
      }
    } catch (e: any) {
      iterOutcome = e.constructor.name;
    }
    console.log("for-await=" + seen.join(",") + ":" + iterOutcome + ",log=" + asyncLog.join(">"));

    const clean: any = {
      [Symbol.asyncIterator]() {
        let i = 0;
        return {
          next(): Promise<any> {
            i++;
            return Promise.resolve({ value: "c" + i, done: i > 3 });
          },
          return(): Promise<any> {
            asyncLog.push("clean-return");
            return Promise.resolve({ done: true });
          },
        };
      },
    };
    let bodyOutcome = "none";
    try {
      for await (const v of clean) {
        void v;
        throw new URIError("body");
      }
    } catch (e: any) {
      bodyOutcome = e.constructor.name;
    }
    console.log("body-outcome=" + bodyOutcome + ",final-log=" + asyncLog.join(">"));
  }

  main().then(() => {
    console.log("async-tail=reached");
  });
}

// 9. Iterator closing: return() is called on break, return and body throws; when
// both the body and return() throw, the body's error escapes.
{
  const log: string[] = [];
  function makeIterable(name: string, opts: any): any {
    let i = 0;
    return {
      [Symbol.iterator]() {
        log.push(name + ".iterator");
        return {
          next() {
            log.push(name + ".next" + i);
            if (opts.nextThrowsAt === i) {
              throw new RangeError("next-" + name);
            }
            i++;
            return i > 3 ? { value: undefined, done: true } : { value: name + i, done: false };
          },
          return(v: any) {
            log.push(name + ".return(" + String(v) + ")");
            if (opts.returnThrows) {
              throw new EvalError("return-" + name);
            }
            if (opts.returnBadResult) {
              return 1 as any;
            }
            return { value: "closed", done: true };
          },
        };
      },
    };
  }
  function reset(): void {
    log.length = 0;
  }

  reset();
  console.log("break=" + probe(() => {
    const out: string[] = [];
    for (const v of makeIterable("b", {})) {
      out.push(v);
      if (out.length === 2) break;
    }
    return out.join(",");
  }));
  console.log("break-log=" + log.join("|"));

  reset();
  console.log("return=" + probe(() => {
    for (const v of makeIterable("r", {})) {
      return "returned-" + v;
    }
    return "never";
  }));
  console.log("return-log=" + log.join("|"));

  reset();
  console.log("body-throw=" + probe(() => {
    for (const v of makeIterable("t", {})) {
      throw new URIError("body-" + v);
    }
    return "never";
  }));
  console.log("body-throw-log=" + log.join("|"));

  reset();
  console.log("both-throw=" + probe(() => {
    for (const v of makeIterable("x", { returnThrows: true })) {
      throw new URIError("body-" + v);
    }
    return "never";
  }));
  console.log("both-throw-log=" + log.join("|"));

  reset();
  console.log("break-return-throws=" + probe(() => {
    for (const v of makeIterable("y", { returnThrows: true })) {
      void v;
      break;
    }
    return "finished";
  }));
  console.log("break-return-throws-log=" + log.join("|"));

  reset();
  console.log("bad-return-result=" + probe(() => {
    for (const v of makeIterable("z", { returnBadResult: true })) {
      void v;
      break;
    }
    return "finished";
  }));

  reset();
  console.log("next-throws=" + probe(() => {
    const out: string[] = [];
    for (const v of makeIterable("n", { nextThrowsAt: 2 })) {
      out.push(v);
    }
    return out.join(",");
  }));
  console.log("next-throws-log=" + log.join("|"));

  reset();
  console.log("destructure-partial=" + probe(() => {
    const [first, second] = makeIterable("d", {});
    return first + "," + second;
  }));
  console.log("destructure-partial-log=" + log.join("|"));

  reset();
  console.log("destructure-default-throws=" + probe(() => {
    const [first, second = (() => {
      throw new SyntaxError("default");
    })()] = makeIterable("f", { nextThrowsAt: 5 });
    return String(first) + String(second);
  }));
  console.log("destructure-default-log=" + log.join("|"));

  reset();
  console.log("from-mapper-throws=" + probe(() => Array.from(makeIterable("m", {}), () => {
    throw new URIError("mapper");
  })));
  console.log("from-mapper-log=" + log.join("|"));

  const genLog: string[] = [];
  function* guardedGen(): Generator<number, void, undefined> {
    try {
      genLog.push("g-1");
      yield 1;
      genLog.push("g-2");
      yield 2;
    } finally {
      genLog.push("g-finally");
    }
  }
  for (const v of guardedGen()) {
    genLog.push("body-" + v);
    break;
  }
  console.log("generator=" + genLog.join("|"));
}
