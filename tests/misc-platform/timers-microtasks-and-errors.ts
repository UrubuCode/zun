// Cross-runtime: the event-loop contract and the error surface.
//   - one microtask queue shared by queueMicrotask and promise reactions (FIFO),
//     drained completely before any timer or immediate callback runs;
//   - setTimeout forwarding its extra arguments, clearTimeout idempotence,
//     nested timers landing in a later turn, setInterval until cleared;
//   - AggregateError, Error.cause, DOMException fields, legacy codes, subclassing.
// Ordering claims are only made where the spec fixes them. setTimeout(0) and
// setImmediate race each other from the main module, so that pair is never
// compared; only "microtasks before any macrotask" is asserted for it.

const log: string[] = [];
let finished = false;

function t(f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.constructor.name;
  }
}

console.log("setTimeout_length=" + (setTimeout.length >= 1));
console.log("queueMicrotask_length=" + queueMicrotask.length);

// ---------------------------------------------------------------------------
// 1. Microtasks: one FIFO queue for queueMicrotask and promise reactions.
// ---------------------------------------------------------------------------
queueMicrotask(function () {
  log.push("qm1");
});
Promise.resolve().then(function () {
  log.push("p1");
});
queueMicrotask(function () {
  log.push("qm2");
  queueMicrotask(function () {
    log.push("qm2-nested");
  });
});
Promise.resolve().then(function () {
  log.push("p2");
  return Promise.resolve();
}).then(function () {
  log.push("p2-chained");
});
queueMicrotask(function () {
  log.push("qm3");
});

// A thenable takes an extra turn compared to a plain value.
Promise.resolve({
  then: function (resolve: (v: unknown) => void) {
    log.push("thenable-called");
    resolve(1);
  },
}).then(function () {
  log.push("thenable-settled");
});

// A rejection handled with catch joins the same queue.
Promise.reject(new Error("x")).catch(function () {
  log.push("rejected");
});

// await is a microtask boundary.
(async function (): Promise<void> {
  log.push("async-sync");
  await undefined;
  log.push("after-await-1");
  await undefined;
  log.push("after-await-2");
})();

// setImmediate is a macrotask: it must run after every microtask queued above.
setImmediate(function () {
  log.push("immediate");
});

// ---------------------------------------------------------------------------
// 2. Timers: argument forwarding, clearing, nesting, intervals.
// ---------------------------------------------------------------------------
const handle: any = setTimeout(function (a: unknown, b: unknown, c: unknown) {
  log.push("t0:" + String(a) + "," + String(b) + "," + String(c));
  finish();
}, 0, "x", 7, undefined);

const second: any = setTimeout(function () {
  log.push("t1-should-not-run");
}, 0);
clearTimeout(second);
clearTimeout(second);
clearTimeout(second);
clearTimeout(undefined as any);
clearTimeout(null as any);
clearTimeout(999999 as any);
clearTimeout("not-a-handle" as any);
console.log("clear_is_idempotent=true");
console.log("handle_defined=" + (handle !== undefined) + " truthy=" + Boolean(handle));

setTimeout(function () {
  log.push("no-args:" + arguments.length);
}, 0);
setTimeout(function (...rest: unknown[]) {
  log.push("rest:" + rest.length + ":" + rest.map(String).join("|"));
}, 0, 1, "two", null, false);

// A missing delay is the same as 0.
setTimeout(function () {
  log.push("no-delay");
});

// A microtask queued from INSIDE a timer runs before the next timer.
setTimeout(function () {
  log.push("timerA");
  queueMicrotask(function () {
    log.push("timerA-microtask");
  });
}, 0);
setTimeout(function () {
  log.push("timerB");
}, 0);

// setInterval repeats until cleared, and its handle is cleared the same way.
let ticks = 0;
const interval: any = setInterval(function (label: unknown) {
  ticks++;
  log.push("interval:" + String(label) + ":" + ticks);
  if (ticks === 2) clearInterval(interval);
}, 0, "i");

// A timer scheduled from inside a timer lands in a LATER turn, never this one.
setTimeout(function () {
  log.push("outer");
  setTimeout(function () {
    log.push("inner");
  }, 0);
}, 0);

// A non-function callback is refused, not silently ignored.
try {
  (setTimeout as any)("log.push('never')", 0);
  console.log("string_callback=accepted");
} catch (e: any) {
  console.log("string_callback=" + e.constructor.name);
}
try {
  (setTimeout as any)();
  console.log("no_callback=accepted");
} catch (e: any) {
  console.log("no_callback=" + e.constructor.name);
}

log.push("sync-end");
console.log("nothing_ran_yet=" + JSON.stringify(log));

function finish(): void {
  if (finished) return;
  finished = true;
  // Three turns of slack: the interval is cleared and the timer scheduled from
  // inside a timer has had its own turn before the report runs.
  setTimeout(function () {
    setTimeout(function () {
      setTimeout(report, 0);
    }, 0);
  }, 0);
}

function report(): void {
  const microtaskNames = ["qm1", "p1", "qm2", "qm2-nested", "qm3", "p2", "p2-chained", "thenable-called", "thenable-settled", "rejected", "after-await-1", "after-await-2"];
  const microtasks = log.filter(function (e) {
    return microtaskNames.indexOf(e) >= 0;
  });
  const firstWithPrefix = function (prefix: string): string {
    const hit = log.filter(function (e) {
      return e.indexOf(prefix) === 0;
    });
    return hit.length > 0 ? hit[0] : "<absent>";
  };
  console.log("sync_first=" + (log[0] === "async-sync") + " " + (log[1] === "sync-end"));
  console.log("microtask_order=" + microtasks.join(">"));
  console.log("microtask_count=" + microtasks.length);
  console.log("microtasks_before_immediate=" + (log.indexOf("p2-chained") < log.indexOf("immediate") && log.indexOf("qm2-nested") < log.indexOf("immediate")));
  console.log("microtasks_before_first_timer=" + (log.indexOf("p2-chained") < log.indexOf("t0:x,7,undefined")));
  console.log("timer_after_microtasks=" + (log.indexOf("t0:x,7,undefined") > log.indexOf("qm2-nested")));
  console.log("timer_args=" + firstWithPrefix("t0:"));
  console.log("no_args=" + firstWithPrefix("no-args:"));
  console.log("rest_args=" + firstWithPrefix("rest:"));
  console.log("cleared_never_ran=" + (log.indexOf("t1-should-not-run") < 0));
  console.log("no_delay_ran=" + (log.indexOf("no-delay") >= 0));
  console.log("timer_microtask_before_next_timer=" + (log.indexOf("timerA-microtask") < log.indexOf("timerB")));
  console.log("timers_in_order=" + (log.indexOf("timerA") < log.indexOf("timerB")));
  console.log("interval_ran=" + log.filter(function (e) {
    return e.indexOf("interval:") === 0;
  }).join(" "));
  console.log("interval_repeated_then_stopped=" + (ticks === 2));
  console.log("nested_timer_later=" + (log.indexOf("inner") > log.indexOf("outer")));
  console.log("nested_after_first_round=" + (log.indexOf("inner") > log.indexOf("timerB")));
  console.log("total_entries=" + log.length);
  errorSurface();
}

// ---------------------------------------------------------------------------
// 3. Errors: Error.cause, AggregateError, DOMException.
// ---------------------------------------------------------------------------
function errorSurface(): void {
  const withCause: any = new Error("root", { cause: "origin" });
  console.log("cause=" + String(withCause.cause) + " own=" + Object.prototype.hasOwnProperty.call(withCause, "cause"));
  console.log("cause_absent=" + ("cause" in new Error("plain")));
  console.log("cause_undefined_is_own=" + Object.prototype.hasOwnProperty.call(new Error("u", { cause: undefined }), "cause"));
  console.log("cause_object=" + ((new Error("o", { cause: { k: 1 } }) as any).cause.k));

  const agg = new AggregateError([new Error("a"), "b"], "many");
  console.log("agg_name=" + agg.name + " msg=" + agg.message);
  console.log("agg_len=" + Array.from(agg.errors).length);
  console.log("agg_items=" + Array.from(agg.errors).map(function (x: any) {
    return x instanceof Error ? x.message : String(x);
  }).join("|"));
  console.log("agg_is_error=" + (agg instanceof Error) + " errors_is_array=" + Array.isArray(agg.errors));
  const aggNoMsg = new AggregateError([]);
  console.log("agg_default_msg=" + JSON.stringify(aggNoMsg.message) + " empty=" + aggNoMsg.errors.length);
  const aggWithCause: any = new AggregateError([1], "outer", { cause: "why" });
  console.log("agg_cause=" + aggWithCause.cause);

  // DOMException: name, message, code; defaults and coercion.
  const d = new DOMException("nope", "AbortError");
  console.log("dom_fields=" + d.name + "|" + d.message + "|" + d.code);
  console.log("dom_defaults=" + new DOMException().name + "|" + JSON.stringify(new DOMException().message) + "|" + new DOMException().code);
  console.log("dom_message_only=" + new DOMException("boom").name + "|" + new DOMException("boom").code);
  console.log("dom_unknown_name=" + new DOMException("m", "NotARealName").name + " code=" + new DOMException("m", "NotARealName").code);
  console.log("dom_empty_strings=" + JSON.stringify(new DOMException("", "").name) + "|" + JSON.stringify(new DOMException("", "").message));
  console.log("dom_name_coerced=" + new DOMException("m", 5 as any).name + "|" + typeof new DOMException("m", 5 as any).name);
  console.log("dom_message_coerced=" + JSON.stringify(new DOMException(123 as any).message) + "|" + JSON.stringify(new DOMException(null as any).message));
  console.log("dom_throwing_argument=" + (function (): string {
    try {
      new DOMException({ toString: function (): string { throw new RangeError("x"); } } as any);
      return "no-throw";
    } catch (e: any) {
      return e.constructor.name;
    }
  })());

  // Identity and prototype wiring.
  console.log("dom_is_error=" + (d instanceof Error) + "," + (d instanceof DOMException));
  console.log("dom_proto_chain=" + (Object.getPrototypeOf(DOMException.prototype) === Error.prototype) + "/" + (Object.getPrototypeOf(DOMException) === (Error as any)));
  console.log("dom_constructor_link=" + (DOMException.prototype.constructor === DOMException) + "/" + (new DOMException().constructor === DOMException));
  console.log("dom_tag=" + Object.prototype.toString.call(d) + " error_tag=" + Object.prototype.toString.call(new Error()));
  console.log("dom_tostring=" + String(d) + " | " + String(new DOMException()) + " | " + String(new DOMException("only")));
  console.log("dom_tostring_is_error=" + (DOMException.prototype.toString === Error.prototype.toString));
  console.log("dom_ctor_shape=" + typeof DOMException + " name=" + DOMException.name + " length=" + DOMException.length);
  console.log("dom_call_without_new=" + t(function () { return (DOMException as any)("m"); }));

  // name, message and code are accessors on the prototype, not own data.
  console.log("dom_accessors=" + ["name", "message", "code"].map(function (k) {
    const desc: any = Object.getOwnPropertyDescriptor(DOMException.prototype, k);
    return k + ":" + (desc ? (desc.get ? "getter" : "data") : "absent");
  }).join(","));
  console.log("dom_accessor_attrs=" + (function (): string {
    const desc: any = Object.getOwnPropertyDescriptor(DOMException.prototype, "name");
    return "set:" + String(desc.set) + " e:" + desc.enumerable + " c:" + desc.configurable;
  })());
  console.log("dom_getter_on_plain=" + (function (): string {
    const desc: any = Object.getOwnPropertyDescriptor(DOMException.prototype, "name");
    try {
      return String(desc.get.call({}));
    } catch (e: any) {
      return "throw:" + e.constructor.name;
    }
  })());
  console.log("dom_no_own_fields=" + Object.prototype.hasOwnProperty.call(new DOMException("m", "X"), "name") + "/" + Object.prototype.hasOwnProperty.call(new DOMException("m", "X"), "message"));
  console.log("error_own_fields=" + Object.prototype.hasOwnProperty.call(new Error("m"), "message") + "/" + Object.prototype.hasOwnProperty.call(new Error("m"), "name"));
  console.log("dom_own_enumerable=" + JSON.stringify(Object.keys(d)));

  // Legacy numeric codes: a fixed table keyed by name; newer names carry 0.
  const namedWithCode: string[] = [
    "IndexSizeError", "HierarchyRequestError", "WrongDocumentError", "InvalidCharacterError",
    "NoModificationAllowedError", "NotFoundError", "NotSupportedError", "InUseAttributeError",
    "InvalidStateError", "SyntaxError", "InvalidModificationError", "NamespaceError",
    "InvalidAccessError", "TypeMismatchError", "SecurityError", "NetworkError", "AbortError",
    "URLMismatchError", "QuotaExceededError", "TimeoutError", "InvalidNodeTypeError", "DataCloneError",
  ];
  console.log("codes=" + namedWithCode.map(function (n) {
    return n + ":" + new DOMException("x", n).code;
  }).join(" "));
  const codeless: string[] = ["EncodingError", "NotReadableError", "UnknownError", "ConstraintError", "DataError", "TransactionInactiveError", "ReadOnlyError", "VersionError", "OperationError", "NotAllowedError"];
  console.log("codeless=" + codeless.map(function (n) {
    return new DOMException("x", n).code;
  }).join(","));
  const consts: string[] = ["INDEX_SIZE_ERR", "DOMSTRING_SIZE_ERR", "HIERARCHY_REQUEST_ERR", "WRONG_DOCUMENT_ERR", "INVALID_CHARACTER_ERR", "NO_DATA_ALLOWED_ERR", "NO_MODIFICATION_ALLOWED_ERR", "NOT_FOUND_ERR", "NOT_SUPPORTED_ERR", "INUSE_ATTRIBUTE_ERR", "INVALID_STATE_ERR", "SYNTAX_ERR", "INVALID_MODIFICATION_ERR", "NAMESPACE_ERR", "INVALID_ACCESS_ERR", "VALIDATION_ERR", "TYPE_MISMATCH_ERR", "SECURITY_ERR", "NETWORK_ERR", "ABORT_ERR", "URL_MISMATCH_ERR", "QUOTA_EXCEEDED_ERR", "TIMEOUT_ERR", "INVALID_NODE_TYPE_ERR", "DATA_CLONE_ERR"];
  console.log("const_pairs=" + consts.map(function (k) {
    const ctorVal = (DOMException as any)[k];
    const protoVal = (DOMException.prototype as any)[k];
    return k + ":" + ctorVal + (ctorVal === protoVal ? "=" : "!");
  }).join(" "));
  const constDesc: any = Object.getOwnPropertyDescriptor(DOMException, "ABORT_ERR");
  console.log("const_desc=w:" + constDesc.writable + " e:" + constDesc.enumerable + " c:" + constDesc.configurable);
  console.log("const_matches_code=" + (new DOMException("x", "AbortError").code === (DOMException as any).ABORT_ERR));

  // Subclassing: the accessors read internal state, so subclasses keep working.
  console.log("dom_subclass=" + (function (): string {
    class MyAbort extends DOMException {
      constructor() {
        super("stopped", "AbortError");
      }
    }
    const e = new MyAbort();
    return e.name + "/" + e.message + "/" + e.code + "/" + (e instanceof DOMException) + "/" + (e instanceof Error) + "/" + e.constructor.name;
  })());
  console.log("dom_subclass_tag=" + (function (): string {
    class MyAbort extends DOMException {}
    return Object.prototype.toString.call(new MyAbort());
  })());
  console.log("dom_catchable=" + (function (): string {
    try {
      throw new DOMException("thrown", "DataCloneError");
    } catch (e: any) {
      return e.name + "/" + e.code + "/" + (e instanceof DOMException);
    }
  })());

  // Real DOMExceptions raised by the platform surface.
  const raised: Array<[string, () => void]> = [
    ["btoa_latin1", function () { btoa("Ā"); }],
    ["atob_alphabet", function () { atob("@"); }],
    ["clone_function", function () { structuredClone(function () { return 1; }); }],
  ];
  for (const r of raised) {
    try {
      r[1]();
      console.log("raised_" + r[0] + "=no-throw");
    } catch (e: any) {
      console.log("raised_" + r[0] + "=" + e.constructor.name + "/" + e.name + "/" + e.code + " isDOMException=" + (e instanceof DOMException));
    }
  }
  const cloned: any = structuredClone(d);
  console.log("dom_cloned=" + cloned.constructor.name + "/" + cloned.name + "/" + cloned.message + "/" + cloned.code + " copy=" + (cloned !== d) + " error=" + (cloned instanceof Error));
}
