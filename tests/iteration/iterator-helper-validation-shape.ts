// Iterator helper contracts that are not about iteration results: eager argument
// validation (a rejected argument closes the source), take/drop count coercion,
// who may call the helpers (generic receivers, branded helper objects), the
// Iterator constructor's abstractness, and where the helpers live on the
// prototype chain.

function out(label: string, value: string): void {
  console.log(label + "=" + value);
}

function attempt(fn: () => unknown): string {
  try {
    return "ok:" + String(fn());
  } catch (e: any) {
    return "threw:" + e.name;
  }
}

const pulls: string[] = [];

function* src(tag: string) {
  try {
    let i = 0;
    while (i < 10) {
      pulls.push(tag + i);
      yield i++;
    }
  } finally {
    pulls.push(tag + "-closed");
  }
}

function* three(tag: string) {
  let i = 1;
  while (i <= 3) {
    pulls.push(tag + i);
    yield i++;
  }
}

// A source that has already produced one value, so its close is observable.
function probeClose(tag: string, build: (g: any) => unknown): void {
  pulls.length = 0;
  const g: any = src(tag);
  g.next();
  pulls.length = 0;
  const result = attempt(() => build(g));
  out(tag, result + " closed=" + pulls.join(","));
}

// ---- A. eager argument validation: refused at the call, source closed ----
probeClose("mapNonCallable", (g) => g.map(42));
probeClose("filterNonCallable", (g) => g.filter(null));
probeClose("flatMapNonCallable", (g) => g.flatMap("nope"));
probeClose("someNonCallable", (g) => g.some({}));
probeClose("everyNonCallable", (g) => g.every(undefined));
probeClose("findNonCallable", (g) => g.find(0));
probeClose("forEachNonCallable", (g) => g.forEach(1));
probeClose("reduceNonCallable", (g) => g.reduce("x"));
probeClose("reduceNonCallableWithSeed", (g) => g.reduce(null, 0));
probeClose("mapNoArgument", (g) => g.map());

// take/drop count: NaN and negatives are RangeError (not TypeError), and close the source
probeClose("takeNaN", (g) => g.take(NaN));
probeClose("takeNegative", (g) => g.take(-1));
probeClose("dropNaN", (g) => g.drop(NaN));
probeClose("dropNegative", (g) => g.drop(-5));
probeClose("takeUndefined", (g) => g.take(undefined));
probeClose("takeSymbol", (g) => g.take(Symbol.iterator));
probeClose("takeValueOfThrows", (g) => g.take({ valueOf() { throw new RangeError("no"); } }));

// ---- B. take/drop count coercion, Infinity, and extra arguments ----
out("takeString", (src("m") as any).take("2").toArray().join(","));
out("takeTrue", (src("n") as any).take(true).toArray().join(","));
out("takeNull", JSON.stringify((src("o") as any).take(null).toArray()));
out("takeFraction", (src("p") as any).take(2.9).toArray().join(","));
out("takeInfinity", (three("r") as any).take(Infinity).toArray().join(","));
out("dropInfinity", JSON.stringify((three("s") as any).drop(Infinity).toArray()));
out("dropPastEnd", JSON.stringify((three("t") as any).drop(99).toArray()));
out("dropZero", (three("u") as any).drop(0).toArray().join(","));
out("takeValueOfNumber", (src("w") as any).take({ valueOf() { return 2; } }).toArray().join(","));
out("takeExtraArgIgnored", (src("y") as any).take(2, "ignored").toArray().join(","));

// ---- C. brand and receiver: helpers are generic over `next`, but the receiver must be an object ----
const IterProto: any = (Iterator as any).prototype;

out("typeofIterator", typeof Iterator);
out("iteratorDirectCall", attempt(() => (Iterator as any)()));
out("iteratorDirectNew", attempt(() => new (Iterator as any)()));

class Counter extends (Iterator as any) {
  i: number;
  limit: number;
  constructor(limit: number) {
    super();
    this.i = 0;
    this.limit = limit;
  }
  next() {
    return this.i < this.limit ? { done: false, value: this.i++ } : { done: true, value: undefined };
  }
}
const counter: any = new Counter(5);
out("subclassHelpers", counter.map((v: number) => v * 2).take(3).toArray().join(","));
out("subclassIsIterator", String(counter instanceof (Iterator as any)));
out("subclassSpread", [...(new Counter(3) as any)].join(","));

const plain: any = { i: 0, next() { return this.i < 3 ? { done: false, value: "p" + this.i++ } : { done: true, value: undefined }; } };
out("genericMapOnPlainNext", IterProto.map.call(plain, (v: string) => v + "!").toArray().join(","));
out("receiverNull", attempt(() => IterProto.map.call(null, (v: any) => v)));
out("receiverNumber", attempt(() => IterProto.map.call(5, (v: any) => v)));
out("receiverString", attempt(() => IterProto.map.call("abc", (v: any) => v)));

const noNext: any = IterProto.map.call({}, (v: any) => v);
out("noNextBuilds", typeof noNext);
out("noNextPull", attempt(() => noNext.next()));

function* gb() {
  yield 1;
  yield 2;
}
const helper: any = gb().map((v: number) => v);
const helperProto = Object.getPrototypeOf(helper);
out("brandedNextForeign", attempt(() => helperProto.next.call({})));
out("brandedReturnForeign", attempt(() => helperProto.return.call({})));
out("brandedNextOnGenerator", attempt(() => helperProto.next.call(gb())));
out("brandedNextOnItself", JSON.stringify(helperProto.next.call(helper)));
const genProto: any = Object.getPrototypeOf(Object.getPrototypeOf(gb()));
out("generatorNextRefusesHelper", attempt(() => genProto.next.call(gb().map((v: any) => v))));

out("symbolIteratorReturnsReceiver", String(IterProto[Symbol.iterator].call(plain) === plain));

const wrappedPlain: any = (Iterator as any).from(plain);
out("fromWrapsPlain", String(wrappedPlain !== plain) + " hasMap=" + typeof wrappedPlain.map);
out("fromKeepsIteratorInstance", String((Iterator as any).from(counter) === counter));
out("fromIterableArray", (Iterator as any).from([1, 2, 3]).toArray().join(","));
out("fromPrimitiveNumber", attempt(() => (Iterator as any).from(5)));
out("fromStringPrimitive", (Iterator as any).from("ab").toArray().join(","));

// `constructor` and @@toStringTag on Iterator.prototype are accessors
const ctorDesc: any = Object.getOwnPropertyDescriptor(IterProto, "constructor");
const tagDesc: any = Object.getOwnPropertyDescriptor(IterProto, Symbol.toStringTag);
out("constructorAccessor", typeof ctorDesc.get + "," + typeof ctorDesc.set);
out("toStringTagAccessor", typeof tagDesc.get + "," + typeof tagDesc.set);
out("toStringTagValue", String(IterProto[Symbol.toStringTag]));
out("constructorValue", String(IterProto.constructor === Iterator));
const receiver: any = Object.create(IterProto);
receiver[Symbol.toStringTag] = "Mine";
out("tagSetOnReceiver", receiver[Symbol.toStringTag] + " own=" + Object.prototype.hasOwnProperty.call(receiver, Symbol.toStringTag));
out("protoTagUntouched", String(IterProto[Symbol.toStringTag]));

// an exhausted helper stays exhausted, and a helper built on it is empty
const spent: any = gb().map((v: number) => v);
spent.toArray();
out("spentNext", JSON.stringify(spent.next()));
out("helperOnSpent", JSON.stringify(spent.map((v: any) => v).toArray()));
const spentGen = gb();
spentGen.next();
spentGen.next();
out("helperOnSpentGenerator", JSON.stringify(spentGen.map((v: any) => v).toArray()));

// ---- D. prototype chain and inventory ----
const helperTag = helper[Symbol.toStringTag];
out("helperTag", helperTag);
out("helperToString", Object.prototype.toString.call(helper));
out("helperTagOwnOnProto", String(Object.prototype.hasOwnProperty.call(helperProto, Symbol.toStringTag)));
out("helperTagOwnOnHelper", String(Object.prototype.hasOwnProperty.call(helper, Symbol.toStringTag)));

const iterProto = Object.getPrototypeOf(helperProto);
const helperKinds = ["map", "filter", "take", "drop", "flatMap"];
const sharedProto = helperKinds.every((k) => {
  const arg: any = k === "take" || k === "drop" ? 1 : (x: any) => x;
  return Object.getPrototypeOf((gb() as any)[k](arg)) === helperProto;
});
out("helpersShareProto", String(sharedProto));
out("helperProtoOwnsNext", String(Object.prototype.hasOwnProperty.call(helperProto, "next")));
out("helperProtoOwnsReturn", String(Object.prototype.hasOwnProperty.call(helperProto, "return")));
out("helperProtoOwnsMap", String(Object.prototype.hasOwnProperty.call(helperProto, "map")));
out("iterProtoIsIteratorPrototype", String(iterProto === (Iterator as any).prototype));
out("iterProtoOwnsSymbolIterator", String(Object.prototype.hasOwnProperty.call(iterProto, Symbol.iterator)));
out("iterProtoParentIsObject", String(Object.getPrototypeOf(iterProto) === Object.prototype));
out("iterProtoConstructorIsIterator", String(iterProto.constructor === Iterator));

const names = ["map", "filter", "take", "drop", "flatMap", "reduce", "toArray", "forEach", "some", "every", "find"];
out("inventoryOwnOnIterProto", String(names.every((k) => Object.prototype.hasOwnProperty.call(iterProto, k))));
out("inventoryAllFunctions", String(names.every((k) => typeof iterProto[k] === "function")));
out("inventoryArities", names.map((k) => k + ":" + iterProto[k].length).join(","));
out("inventoryNonEnumerable", String(names.every((k) => Object.getOwnPropertyDescriptor(iterProto, k).enumerable === false)));

out("generatorChainReachesIterProto", String(Object.getPrototypeOf(genProto) === iterProto));
out("generatorHasMap", typeof gb().map);
out("arrayIteratorHasMap", typeof [1][Symbol.iterator]().map);
out("stringIteratorHasMap", typeof "a"[Symbol.iterator]().map);
out("mapIteratorHasMap", typeof new Map()[Symbol.iterator]().map);
out("setIteratorHasMap", typeof new Set()[Symbol.iterator]().map);
out("matchAllIteratorHasMap", typeof "aa".matchAll(/a/g).map);

const arrIt: any = [1][Symbol.iterator]();
out("symbolIteratorOnIterProtoReturnsThis", String(iterProto[Symbol.iterator].call(arrIt) === arrIt));
out("helperSymbolIteratorReturnsThis", String(helper[Symbol.iterator]() === helper));
out("tagArrayIterator", Object.prototype.toString.call([1][Symbol.iterator]()));
out("tagGenerator", Object.prototype.toString.call(gb()));
out("helperStillWorks", helper.toArray().join(","));
