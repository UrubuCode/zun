// Promise combinators: Promise.all / allSettled / race / any. Covers input-order
// results, thenable entries, empty iterables, misbehaving iterables (throwing
// next(), non-callable Symbol.iterator, non-iterable arguments), settlement
// races, AggregateError shape and error order, the generic constructor protocol
// (one construct, one read of `resolve`, one call per entry), and subclass
// receivers. Every case prints exactly one labelled line.

function out(label: string, value: unknown): void {
  console.log(label + "=" + String(value));
}

function later<T>(depth: number, value: T, reject = false): Promise<T> {
  let p: Promise<unknown> = Promise.resolve();
  for (let i = 0; i < depth; i++) p = p.then(() => undefined);
  return p.then(() => {
    if (reject) throw value;
    return value;
  }) as Promise<T>;
}

function outcome(p: PromiseLike<unknown>): Promise<string> {
  return Promise.resolve(p).then(
    (v) => "fulfilled:" + JSON.stringify(v),
    (e) => "rejected:" + (e instanceof Error ? e.name : String(e)),
  );
}

// A foreign constructor that records how the combinators use it.
function makeCtor() {
  const st: any = { constructs: 0, reads: 0, calls: 0, status: "pending", value: undefined };
  function C(this: any, ex: any) {
    st.constructs++;
    ex(
      (v: unknown) => { if (st.status === "pending") { st.status = "fulfilled"; st.value = v; } },
      (e: unknown) => { if (st.status === "pending") { st.status = "rejected"; st.value = e; } },
    );
  }
  Object.defineProperty(C, "resolve", {
    configurable: true,
    get() {
      st.reads++;
      return (v: unknown) => { st.calls++; return Promise.resolve(v); };
    },
  });
  return { C: C as any, st };
}

class Tracked<T> extends Promise<T> {
  static built = 0;
  constructor(executor: any) {
    super(executor);
    Tracked.built++;
  }
}

async function main(): Promise<void> {
  // ---- Promise.all -------------------------------------------------------
  // 1. Results keep input order even when settlement order is reversed.
  const allOrdered = await Promise.all([later(6, "a"), later(2, "b"), later(4, "c"), "plain"]);
  out("allInputOrder", allOrdered.join(","));
  out("allIsArray", Array.isArray(allOrdered));

  // 2. Thenables and plain values are adopted like promises.
  out("allMixedEntries", (await Promise.all([
    { then(r: (v: string) => void) { r("thenable"); } } as any,
    Promise.resolve("native"),
    7,
  ])).join(","));

  // 3. Empty iterables resolve immediately with an empty array.
  out("allEmpty", JSON.stringify(await Promise.all([])));
  out("allSettledEmpty", JSON.stringify(await Promise.allSettled([])));

  // 4. The first rejection in settlement order decides the rejection.
  out("allFirstRejection", await outcome(Promise.all([later(5, "slow"), later(1, "fast-err", true), later(3, "mid")])));

  // 5. An iterator whose next() throws mid-way rejects the combinator (it does
  //    not throw), and entries already pulled keep running.
  const pulls: string[] = [];
  function badIterable(tag: string) {
    return {
      [Symbol.iterator]() {
        let i = 0;
        return {
          next() {
            i++;
            pulls.push(tag + ":next" + i);
            if (i <= 2) return { value: Promise.resolve(tag + "-v" + i), done: false };
            throw new RangeError("iterator gave up");
          },
          return() {
            pulls.push(tag + ":return");
            return { done: true, value: undefined };
          },
        };
      },
    };
  }
  let threwSync = "no";
  let midFail: Promise<unknown> | undefined;
  try { midFail = Promise.all(badIterable("all") as any); } catch (e) { threwSync = (e as Error).name; }
  out("allMidIteratorThrewSync", threwSync);
  out("allMidIteratorResult", await outcome(midFail!));

  // 6. Non-iterable argument and non-callable Symbol.iterator both reject.
  out("allNonIterable", await outcome(Promise.all(42 as any)));
  out("allNonCallableIterator", await outcome(Promise.all({ [Symbol.iterator]: 7 } as any)));

  // 7. An iterator that throws on its very first pull gives no entries at all.
  const firstPull = {
    [Symbol.iterator]() {
      return {
        next() { pulls.push("first:next"); throw new EvalError("immediate"); },
        return() { pulls.push("first:return"); return { done: true, value: undefined }; },
      };
    },
  };
  out("allFirstPullThrows", await outcome(Promise.all(firstPull as any)));

  // 8. The iterator is drained synchronously, so its throw beats a member that
  //    is already rejected but whose rejection is only observed later.
  const alreadyRejected = Promise.reject(new SyntaxError("member"));
  alreadyRejected.catch(() => {});
  const mixed = {
    [Symbol.iterator]() {
      let i = 0;
      return {
        next() {
          i++;
          pulls.push("mixed:next" + i);
          if (i === 1) return { value: alreadyRejected, done: false };
          throw new URIError("after a rejected member");
        },
      };
    },
  };
  out("allMixedIteratorWins", await outcome(Promise.all(mixed as any)));

  // 9. Per-entry resolve functions are single-shot: a thenable calling twice
  //    contributes its first value only.
  const twice = { then(res: (v: string) => void) { res("first"); res("second"); } };
  out("allSingleShot", JSON.stringify(await Promise.all([twice as any, "plain"])));

  // ---- Promise.allSettled ------------------------------------------------
  // 10. Entry shape, key order and input order regardless of settlement order.
  const settleOrder: string[] = [];
  function tagged(depth: number, value: string, reject: boolean): Promise<unknown> {
    return later(depth, value, reject).then(
      (v) => { settleOrder.push("ok:" + v); return v; },
      (e) => { settleOrder.push("err:" + value); throw e; },
    );
  }
  const rs: any[] = await Promise.allSettled([
    tagged(6, "a", false),
    tagged(4, "b", true),
    tagged(2, "c", false),
    "notAPromise",
    Promise.reject(new TypeError("d")),
  ] as any);
  out("settledLength", rs.length);
  out("settledIsArray", Array.isArray(rs));
  out("settledSettleOrder", settleOrder.join("|"));
  for (let i = 0; i < rs.length; i++) {
    const r = rs[i];
    const payload = r.status === "fulfilled" ? String(r.value) : r.reason.constructor.name;
    out("settled[" + i + "]", Object.keys(r).join(",") + " " + r.status + " " + payload);
  }
  out("settledHasValueOnRejected", "value" in rs[1]);
  out("settledHasReasonOnFulfilled", "reason" in rs[0]);
  out("settledEntryProto", Object.getPrototypeOf(rs[0]) === Object.prototype);
  out("settledDescriptor", [
    Object.getOwnPropertyDescriptor(rs[0], "status")!.writable,
    Object.getOwnPropertyDescriptor(rs[0], "status")!.enumerable,
    Object.getOwnPropertyDescriptor(rs[0], "status")!.configurable,
  ].join(","));

  // ---- Promise.race ------------------------------------------------------
  // 11. The first entry to settle wins; among already-settled entries, input
  //     order decides; a rejection can win; plain values beat pending promises.
  out("raceFirstSettled", await outcome(Promise.race([later(5, "slow"), later(1, "quick"), later(3, "mid")])));
  out("raceInputOrderAmongSettled", await outcome(Promise.race([Promise.resolve("x"), Promise.resolve("y")])));
  out("raceRejectionWins", await outcome(Promise.race([Promise.reject("R"), Promise.resolve("F")])));
  out("racePlainBeatsPending", await outcome(Promise.race([new Promise(() => {}), 99])));
  out("raceThenable", await outcome(Promise.race([
    { then(r: (v: string) => void) { r("T"); } } as any,
    Promise.resolve("N"),
  ])));
  const raceInput = Promise.resolve(1);
  out("raceResultIsNew", Promise.race([raceInput]) !== raceInput);
  out("raceOverString", await outcome(Promise.race("xyz" as any)));
  out("raceOverSet", await outcome(Promise.race(new Set([Promise.resolve("s1"), Promise.resolve("s2")]) as any)));
  let nonIterableRace: Promise<unknown> | undefined;
  let raceThrew = "no";
  try { nonIterableRace = Promise.race({} as any); } catch (e) { raceThrew = "threw"; }
  out("raceNonIterableThrew", raceThrew);
  out("raceNonIterableResult", await outcome(nonIterableRace!));
  out("raceBadIterable", await outcome(Promise.race({
    [Symbol.iterator]() { throw new RangeError("no"); },
  } as any)));

  // 12. Empty and never-settling races stay pending forever.
  let emptyState = "pending";
  Promise.race([]).then(() => { emptyState = "fulfilled"; }, () => { emptyState = "rejected"; });
  let stuckState = "pending";
  Promise.race([new Promise(() => {}), new Promise(() => {})]).then(
    () => { stuckState = "fulfilled"; },
    () => { stuckState = "rejected"; },
  );
  let setState = "pending";
  Promise.race(new Set()).then(() => { setState = "fulfilled"; }, () => { setState = "rejected"; });
  out("raceFreshEachCall", Promise.race([]) !== Promise.race([]));

  // ---- Promise.any -------------------------------------------------------
  // 13. The first fulfilment wins over earlier rejections.
  out("anyLateFulfilment", await outcome(Promise.any([
    Promise.reject(new TypeError("x")),
    Promise.reject(new TypeError("y")),
    Promise.resolve("winner"),
  ])));
  out("anyPlainWins", await outcome(Promise.any(["plain", Promise.reject(new TypeError("z"))])));

  // 14. Empty input rejects with an AggregateError carrying no errors.
  let emptyAny: any;
  try { await Promise.any([]); } catch (e) { emptyAny = e; }
  out("anyEmptyName", emptyAny.name);
  out("anyEmptyIsAggregate", emptyAny instanceof AggregateError);
  out("anyEmptyIsError", emptyAny instanceof Error);
  out("anyEmptyErrorsLength", emptyAny.errors.length);
  out("anyEmptyErrorsIsArray", Array.isArray(emptyAny.errors));
  out("anyErrorsEnumerable", Object.getOwnPropertyDescriptor(emptyAny, "errors")!.enumerable);

  // 15. All rejections: errors keep INPUT order, not rejection order.
  const rejectOrder: string[] = [];
  const kinds: any = { a: RangeError, b: EvalError, c: URIError };
  function lateReject(depth: number, tag: string): Promise<unknown> {
    return later(depth, tag).then(() => {
      rejectOrder.push(tag);
      throw new kinds[tag]("late");
    });
  }
  let allRejected: any;
  try { await Promise.any([lateReject(6, "a"), lateReject(2, "b"), lateReject(4, "c")]); } catch (e) { allRejected = e; }
  out("anyAllRejectedName", allRejected.name);
  out("anyInputOrder", allRejected.errors.map((x: any) => x.constructor.name).join(","));
  out("anySettleOrder", rejectOrder.join(","));

  // 16. A thenable whose then rejects participates as a rejection.
  let thenRej: any;
  try {
    await Promise.any([{ then(_r: any, j: (e: string) => void) { j("c"); } } as any]);
  } catch (e) { thenRej = e; }
  out("anyThenableRejection", thenRej.errors.join(","));

  // 17. AggregateError built by hand.
  const hand = new AggregateError([1, 2, 3], "msg");
  out("handShape", hand.name + "," + hand.errors.length + "," + hand.message);
  out("handProtoChain", Object.getPrototypeOf(AggregateError.prototype) === Error.prototype);

  // ---- Generic constructor protocol --------------------------------------
  // 18. Each combinator reads `resolve` once per call and calls it once per
  //     entry; the result is built with `new this(executor)`.
  const a = makeCtor();
  const rAll = (Promise.all as any).call(a.C, [1, 2, 3]);
  out("ctorAllInstance", rAll instanceof a.C);
  out("ctorAllNative", rAll instanceof Promise);
  out("ctorAll", a.st.constructs + "," + a.st.reads + "," + a.st.calls);
  const b = makeCtor();
  (Promise.race as any).call(b.C, [1, 2]);
  out("ctorRace", b.st.constructs + "," + b.st.reads + "," + b.st.calls);
  const c = makeCtor();
  (Promise.allSettled as any).call(c.C, [1, 2, 3, 4]);
  out("ctorAllSettled", c.st.constructs + "," + c.st.reads + "," + c.st.calls);
  const d = makeCtor();
  (Promise.any as any).call(d.C, [1, 2]);
  out("ctorAny", d.st.constructs + "," + d.st.reads + "," + d.st.calls);
  const e = makeCtor();
  (Promise.all as any).call(e.C, []);
  out("ctorEmptyAll", e.st.constructs + "," + e.st.reads + "," + e.st.calls + "," + e.st.status);
  // Promise.resolve / reject on a foreign constructor never touch `resolve`.
  const f = makeCtor();
  const rf = (Promise.resolve as any).call(f.C, 7);
  out("ctorResolveForeign", (rf instanceof f.C) + "," + f.st.constructs + "," + f.st.reads + "," + f.st.value);
  // A non-callable `resolve` on the receiver rejects the capability.
  const h = makeCtor();
  Object.defineProperty(h.C, "resolve", { configurable: true, value: 42 });
  let hThrew = "no";
  try { (Promise.all as any).call(h.C, [1]); } catch (err) { hThrew = (err as Error).name; }
  out("ctorBadResolve", hThrew + "," + h.st.status + "," + (h.st.value instanceof TypeError));
  // A non-constructor receiver throws synchronously.
  let nonCtor = "no";
  try { (Promise.all as any).call({}, [1]); } catch (err) { nonCtor = (err as Error).name; }
  out("ctorNonConstructor", nonCtor);

  // ---- Subclass receivers ------------------------------------------------
  // 19. Combinators build with the receiver, so a subclass gets its own type.
  Tracked.built = 0;
  const trackedAll = Tracked.all([1, 2]) as any;
  out("subAllInstance", trackedAll instanceof Tracked);
  out("subAllBuilt", Tracked.built);
  Tracked.built = 0;
  const trackedRace = Tracked.race([Promise.resolve(1)]) as any;
  out("subRaceInstance", trackedRace instanceof Tracked);
  const trackedSettled = Tracked.allSettled([1]) as any;
  out("subAllSettledInstance", trackedSettled instanceof Tracked);
  out("subAllValues", await trackedAll);
  out("subSettledValues", JSON.stringify(await trackedSettled));

  // 20. Drain past everything above and report the pending states and pulls.
  for (let i = 0; i < 24; i++) await null;
  out("emptyRaceState", emptyState);
  out("stuckRaceState", stuckState);
  out("setRaceState", setState);
  out("pulls", pulls.join("|"));
  out("pullCount", pulls.length);
  out("ctorAllFinalStatus", a.st.status + "," + JSON.stringify(a.st.value));
  console.log("end");
}

main().catch((e) => console.log("UNEXPECTED " + String(e)));
