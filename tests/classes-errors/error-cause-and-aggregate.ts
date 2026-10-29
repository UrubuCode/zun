"use strict";
// Error.cause (presence rules, descriptor, options coercion, subclass forwarding,
// cycles and chains) and AggregateError (argument order, errors array shape,
// iterable consumption, optional message, subclassing) plus the Promise
// combinators that produce rejections. Only constructor names, messages the
// author wrote, booleans and descriptor flags are printed.

function desc(o: any, key: string): string {
  const d: any = Object.getOwnPropertyDescriptor(o, key);
  return "w" + d.writable + ",e" + d.enumerable + ",c" + d.configurable;
}
function ownFlag(o: any, key: string): string {
  return String(Object.prototype.hasOwnProperty.call(o, key));
}
function probe(fn: () => any): string {
  try {
    const v = fn();
    return "ok:" + String(v);
  } catch (e: any) {
    return e.constructor.name;
  }
}
function report(e: any, tag: string): void {
  const has = Object.prototype.hasOwnProperty.call(e, "cause");
  console.log(tag + "-own=" + has + ",in=" + ("cause" in e) + ",value=" + String(e.cause));
}

// 1. When is cause installed? Only when the options object HAS the key.
{
  report(new Error("a"), "no-options");
  report(new Error("a", {}), "empty-options");
  report(new Error("a", { cause: 42 }), "with-cause");
  report(new Error("a", { cause: undefined }), "undefined-cause");
  report(new Error("a", 5 as any), "number-options");
  report(new Error("a", null as any), "null-options");
  report(new Error("a", "cause" as any), "string-options");
  const protoOpts: any = Object.create({ cause: "inherited" });
  report(new Error("a", protoOpts), "inherited-options");

  let reads = 0;
  const getterOpts: any = {};
  Object.defineProperty(getterOpts, "cause", {
    get() {
      reads = reads + 1;
      return "lazy";
    },
    enumerable: true,
    configurable: true,
  });
  const lazy = new Error("a", getterOpts);
  console.log("getter-reads=" + reads + ",value=" + lazy.cause + ",after=" + reads);

  console.log("cause-desc=" + desc(new Error("a", { cause: 1 }), "cause"));
  console.log("cause-desc-value=" + (Object.getOwnPropertyDescriptor(new Error("a", { cause: 1 }), "cause") as any).value);
}

// 2. Non-enumerable, invisible to JSON and keys; chain traversal.
{
  const withCause = new Error("outer", { cause: new Error("inner") });
  console.log("keys=" + Object.keys(withCause).length + ",json=" + JSON.stringify(withCause));
  console.log("chain=" + (withCause.cause as any).message);

  const leaf = new RangeError("leaf");
  const mid = new Error("mid", { cause: leaf });
  const top = new TypeError("top", { cause: mid });
  console.log("chain3=" + top.message + ">" + (top.cause as any).message + ">" + (top.cause as any).cause.message);
  console.log("chain3-names=" + top.name + ">" + (top.cause as any).name + ">" + (top.cause as any).cause.name);

  const selfRef: any = new Error("self");
  const wrapper: any = new Error("wrap", { cause: selfRef });
  selfRef.cause = wrapper;
  console.log("cycle=" + (wrapper.cause.cause === wrapper));
  console.log("primitive-cause=" + new Error("p", { cause: null }).cause + ",false=" + new Error("p", { cause: false }).cause);
}

// 3. Cause on every Error subclass; AggregateError takes options third.
{
  report(new TypeError("t", { cause: "t" }), "typeerror");
  report(new RangeError("r", { cause: "r" }), "rangeerror");
  report(new SyntaxError("s", { cause: "s" }), "syntaxerror");
  report(new ReferenceError("f", { cause: "f" }), "referenceerror");
  report(new EvalError("v", { cause: "v" }), "evalerror");
  report(new URIError("u", { cause: "u" }), "urierror");
  const agg = new AggregateError([new Error("x")], "agg", { cause: "aggcause" });
  report(agg, "aggregate");
  console.log("agg-message=" + agg.message + ",errors=" + agg.errors.length);
}

// 4. User subclasses forwarding options: the subclass's own fields stay enumerable.
{
  class AppError extends Error {
    code: string;
    constructor(message: string, code: string, options?: any) {
      super(message, options);
      this.code = code;
    }
  }
  const app = new AppError("app", "E1", { cause: "root" });
  report(app, "subclass");
  console.log("subclass=" + app.code + "," + app.message + "," + app.name + ",keys=" + Object.keys(app).join(","));
  report(new AppError("app", "E2"), "subclass-none");

  class Named extends Error {
    constructor(message: string, options?: any) {
      super(message, options);
      this.name = "Named";
    }
  }
  const nm = new Named("n", { cause: 7 });
  console.log("named-cause=" + nm.cause + ",name=" + nm.name + ",own-cause=" + ownFlag(nm, "cause"));
}

// 5. AggregateError: argument order, errors array shape, descriptor.
{
  const a = new AggregateError([new TypeError("t"), new RangeError("r")], "two");
  console.log("agg-name=" + a.name + ",message=" + a.message + ",len=" + a.errors.length + ",isArray=" + Array.isArray(a.errors));
  console.log("agg-kinds=" + a.errors[0].constructor.name + "," + a.errors[1].constructor.name);
  console.log("agg-instanceof=" + (a instanceof Error) + ":" + (a instanceof AggregateError) + ",tag=" + Object.prototype.toString.call(a));
  console.log("errors-desc=" + desc(a, "errors") + ",own=" + ownFlag(a, "errors") + ",on-proto=" + ownFlag(AggregateError.prototype, "errors"));
  console.log("agg-keys=" + Object.keys(a).length + ",json=" + JSON.stringify(a));

  const noMsg = new AggregateError([]);
  console.log("nomsg=" + ownFlag(noMsg, "message") + ":" + JSON.stringify(noMsg.message) + ",tostring=" + noMsg.toString() + ",len=" + noMsg.errors.length);
  console.log("undef-msg-own=" + ownFlag(new AggregateError([], undefined), "message"));

  const fromSet: any = new AggregateError(new Set(["a", "b", "c"]) as any, "set");
  console.log("set=" + fromSet.errors.length + ":" + fromSet.errors.join(",") + ",plain=" + (Object.getPrototypeOf(fromSet.errors) === Array.prototype));

  function* two(): Generator<number, void, undefined> {
    yield 1;
    yield 2;
  }
  console.log("gen=" + (new AggregateError(two() as any, "gen") as any).errors.join(","));
  console.log("string=" + (new AggregateError("hey" as any, "str") as any).errors.join("|"));

  const src: any[] = [1, 2];
  const copied: any = new AggregateError(src, "copy");
  src.push(3);
  console.log("copied=" + copied.errors.length + ",same=" + (copied.errors === src));

  console.log("non-iterable=" + probe(() => new AggregateError(5 as any, "bad")));
  console.log("undefined-errors=" + probe(() => new AggregateError(undefined as any)));
  console.log("null-errors=" + probe(() => new AggregateError(null as any)));

  const called: any = (AggregateError as any)([new Error("x")], "called");
  console.log("called=" + (called instanceof AggregateError) + ":" + called.errors.length);

  class Batch extends AggregateError {
    label: string;
    constructor(errs: any[], label: string) {
      super(errs, "batch:" + label);
      this.label = label;
    }
  }
  const b = new Batch([new Error("q")], "L");
  console.log("batch=" + b.name + "|" + b.message + "|" + b.label + "|" + b.errors.length + "|" + Object.keys(b).join(","));
  console.log("batch-instanceof=" + (b instanceof AggregateError) + ":" + (b instanceof Error));
}

// 6. The array is a fresh copy of the iterable, so the source can keep changing.
{
  const copyOf: any = new AggregateError([1, 2], "x");
  console.log("copy-plain=" + (Object.getPrototypeOf(copyOf.errors) === Array.prototype));
}

// 7. Promise combinators: first reason, reason list, and the AggregateError from any.
const combinator: string[] = [];
Promise.all([Promise.resolve(1), Promise.reject(new TypeError("first")), Promise.reject(new RangeError("second"))])
  .then(
    () => {
      combinator.push("all-resolved");
    },
    (e: any) => {
      combinator.push("all=" + e.constructor.name + ":" + e.message);
    },
  )
  .then(() =>
    Promise.race([Promise.reject(new EvalError("race")), Promise.resolve("later")]).then(
      () => {
        combinator.push("race-resolved");
      },
      (e: any) => {
        combinator.push("race=" + e.constructor.name);
      },
    ),
  )
  .then(() =>
    Promise.allSettled([Promise.resolve("v"), Promise.reject(new URIError("s"))]).then((rs: any[]) => {
      combinator.push("settled=" + rs.length + "," + rs[0].status + ":" + rs[0].value + "," + rs[1].status + ":" + rs[1].reason.constructor.name);
      combinator.push("settled-keys=" + Object.keys(rs[0]).join(",") + "|" + Object.keys(rs[1]).join(","));
    }),
  )
  .then(() =>
    Promise.any([Promise.reject(new TypeError("a")), Promise.resolve("won"), Promise.reject(new RangeError("b"))]).then(
      (v: any) => {
        combinator.push("any=" + v);
      },
    ),
  )
  .then(() =>
    Promise.any([Promise.reject(new TypeError("a")), Promise.reject(new RangeError("b"))]).then(
      () => {
        combinator.push("any2-resolved");
      },
      (e: any) => {
        combinator.push(
          "any2=" +
            e.constructor.name +
            ":" +
            e.errors.length +
            ":" +
            e.errors.map((x: any) => x.constructor.name).join(",") +
            ":" +
            (e instanceof AggregateError) +
            ":" +
            (e instanceof Error),
        );
      },
    ),
  )
  .then(() => {
    for (const line of combinator) {
      console.log("combinator " + line);
    }
  });
