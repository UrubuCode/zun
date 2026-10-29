// The `arguments` object and the many-argument call paths. Strict code only:
// there the object is always unmapped, so writes never cross between the
// parameters and `arguments`. Covers shape and iterability, absence versus
// undefined, callee/caller poisoning, arrows and nested functions, copies and
// escapes, and then spread calls, apply with large lists, holes, iterables,
// evaluation order, non-iterable refusals and rest collection.
export {};

const show = (label: string, value: unknown): void => {
  console.log(label + "=" + JSON.stringify(value));
};

function tryOf(fn: () => unknown): string {
  try {
    return "ok:" + String(fn());
  } catch (e) {
    return "threw:" + (e as any).name;
  }
}

// ---------------------------------------------------------------------------
// 1. Shape and iteration
// ---------------------------------------------------------------------------
function sum(): number {
  let total = 0;
  for (let i = 0; i < arguments.length; i++) total += arguments[i];
  return total;
}
show("sum", sum(1, 2, 3, 4, 5));

function shape(a: number = 0): string {
  return [
    "isArray=" + Array.isArray(arguments),
    "typeof=" + typeof arguments,
    "tag=" + Object.prototype.toString.call(arguments),
    "hasMap=" + typeof (arguments as any).map,
    "length=" + arguments.length,
  ].join(" ");
}
show("shape", shape(1, 2 as any));

function iterates(a: number = 0): string {
  const collected: string[] = [];
  for (const v of arguments as any) collected.push(String(v));
  const spread = [...(arguments as any)];
  const converted = Array.from(arguments as any);
  return collected.join(",") + "|" + spread.join(",") + "|" + converted.length +
    "|sameIterator=" + ((arguments as any)[Symbol.iterator] === Array.prototype.values);
}
show("iterates", iterates(1, 2 as any, 3 as any));

function absence(a: number, b: number = 1): string {
  return "has0=" + (0 in arguments) + " has1=" + (1 in arguments) +
    " keys=" + Object.keys(arguments).join(",");
}
show("absence", absence(1));

function countsByLength(a: number, b: number = 0, c: number = 0): string {
  return "passed=" + arguments.length + " declared=" + countsByLength.length;
}
show("counts_one", countsByLength(1));
show("counts_three", countsByLength(1, 2 as any, 3 as any));
show("counts_extra", countsByLength(1, 2 as any, 3 as any, 4 as any, 5 as any));

function indexAttrs(a: number): string {
  const d: any = Object.getOwnPropertyDescriptor(arguments, "0");
  return "w=" + d.writable + " e=" + d.enumerable + " c=" + d.configurable;
}
show("index_attrs", indexAttrs(1));

function lengthWritable(a: number): string {
  const accepted = Reflect.set(arguments, "length", 9);
  return "accepted=" + accepted + " value=" + arguments.length;
}
show("length_writable", lengthWritable(1));

function deletes(a: number): string {
  const accepted = Reflect.deleteProperty(arguments, "0");
  return "accepted=" + accepted + " has0=" + (0 in arguments) + " length=" + arguments.length;
}
show("deletes", deletes(1, 2 as any));

function beyond(a: number): string {
  const startLength = arguments.length;
  (arguments as any)[5] = "far";
  return "start=" + startLength + " after=" + arguments.length + " read=" + (arguments as any)[5];
}
show("beyond", beyond(1));

// ---------------------------------------------------------------------------
// 2. Unmapped in strict code: no aliasing either way
// ---------------------------------------------------------------------------
function unmapped(a: number, b: number): string {
  arguments[0] = 99;
  const afterIndexWrite = String(a);
  a = 7;
  const afterParamWrite = String(arguments[0]);
  return afterIndexWrite + "/" + afterParamWrite + "/" + b;
}
show("unmapped", unmapped(1, 2));

function sloppyStyle(a: any, b: any): string {
  arguments[0] = "A";
  b = "B";
  return a + ":" + arguments[1];
}
show("no_alias_b", sloppyStyle("a", "b"));

class Strictly {
  probe(a: number): string {
    arguments[0] = 100;
    const seen = a;
    a = 3;
    return seen + "|" + String(arguments[0]);
  }
}
show("class_method_unmapped", new Strictly().probe(1));

// callee is poisoned in strict code, and so are caller and arguments on functions.
function callee(): boolean {
  let threw = false;
  try { void (arguments as any).callee; } catch (e) { threw = e instanceof TypeError; }
  return threw;
}
show("callee_poisoned", callee());
function inspectCallerAccess(): boolean[] {
  const results: boolean[] = [];
  try { void (inspectCallerAccess as any).caller; } catch (e) { results.push(e instanceof TypeError); }
  return results;
}
show("caller_poisoned", inspectCallerAccess());

// ---------------------------------------------------------------------------
// 3. Arrows and nesting
// ---------------------------------------------------------------------------
function outerArgs(a: number = 0): string {
  const inner = (): string => String(arguments.length) + ":" + String((arguments as any)[0]);
  return inner();
}
show("arrow_sees_outer", outerArgs(7, 8 as any));

function nestedOwn(a: number = 0): string {
  function inner(x: number = 0): string {
    return "inner=" + arguments.length;
  }
  return inner(1, 2 as any) + " outer=" + arguments.length;
}
show("nested_own", nestedOwn(1, 2 as any, 3 as any));

function copied(a: number = 0): string {
  const snapshot = Array.prototype.slice.call(arguments);
  (arguments as any)[0] = "changed";
  return snapshot.join(",") + "|live=" + String((arguments as any)[0]);
}
show("copied", copied(1, 2 as any));

function escapes(a: number = 0): any { return arguments; }
const escaped: any = escapes(1, 2 as any);
show("escaped", [escaped.length, escaped[0], escaped[1]]);
show("distinct_objects", escapes(1) !== escapes(1));

function defaultInvisible(a: number, b: number = 500): string {
  return "b=" + b + " arguments1=" + String((arguments as any)[1]) + " length=" + arguments.length;
}
show("default_invisible", defaultInvisible(1));

function* generated(a: number = 0): Generator<string> {
  yield "len=" + arguments.length;
  (arguments as any)[0] = "gen";
  yield "a=" + a + " arg0=" + String((arguments as any)[0]);
}
const g = generated(1, 2 as any);
show("generator_first", g.next().value);
show("generator_second", g.next().value);

// rest parameters versus arguments
function withRest(first: number, ...rest: number[]): number[] {
  return [first, rest.length, arguments.length];
}
show("rest_vs_arguments", withRest(1, 2, 3, 4));

function applyOld(fn: Function): number {
  const args = Array.prototype.slice.call(arguments, 1);
  return fn.apply(null, args);
}
show("apply_old", applyOld(Math.max, 3, 1, 4, 1, 5));

// ---------------------------------------------------------------------------
// 4. Spread and apply with many arguments
// ---------------------------------------------------------------------------
function sumRest(...values: number[]): number {
  let total = 0;
  for (let i = 0; i < values.length; i++) total += values[i];
  return total;
}
function countArgs(): number { return arguments.length; }
function firstLast(...values: any[]): string {
  return values.length + ":" + String(values[0]) + ".." + String(values[values.length - 1]);
}

const many: number[] = [];
for (let i = 1; i <= 128; i++) many.push(i);

show("spread_sum", sumRest(...many));
show("spread_shape", firstLast(...many));
show("apply_sum", sumRest.apply(null, many));
show("apply_shape", firstLast.apply(null, many));
show("spread_equals_apply", sumRest(...many) === sumRest.apply(null, many));
show("max_spread", Math.max(...many));
show("max_apply", Math.max.apply(null, many));
show("min_spread", Math.min(...many));

show("interleaved", firstLast(0, ...[1, 2], 3, ...[4, 5]));
show("interleaved_sum", sumRest(1, ...[2, 3], 4, ...[5]));
show("spread_middle", firstLast("a", ...["b"], "c"));
const small = [7, 8];
show("double_spread", sumRest(...small, ...small));

show("empty_array", countArgs(...[]));
show("empty_string", countArgs(..."" as any));
show("empty_set", countArgs(...new Set()));
show("empty_between", firstLast(1, ...[], 2));

show("spread_string", firstLast(..."abc"));
show("spread_set", sumRest(...new Set([1, 2, 3])));
show("spread_map_values", sumRest(...new Map([["a", 4], ["b", 6]]).values()));
function* threeValues(): Generator<number> { yield 10; yield 20; yield 30; }
show("spread_generator", sumRest(...threeValues()));
show("spread_holes", countArgs(...[1, , 3] as any));
show("spread_hole_value", firstLast(...[1, , 3] as any));

// Operands are evaluated left to right; a non-iterable spread is a TypeError
// raised after the fixed operands before it were evaluated.
const evaluated: string[] = [];
function note<T>(label: string, v: T): T {
  evaluated.push(label);
  return v;
}
show("non_iterable", tryOf(() => countArgs(note("fixed", 1), ...(note("bad", 42) as any))));
show("non_iterable_order", evaluated);
evaluated.length = 0;
countArgs(note("a", 1), ...(note("b", [2, 3]) as any), note("c", 4));
show("eval_order", evaluated);

// The spread source is iterated once, at call time.
let nextCalls = 0;
const countingIterable: any = {
  [Symbol.iterator]() {
    let i = 0;
    return {
      next() {
        nextCalls += 1;
        return i < 3 ? { value: i++, done: false } : { value: undefined, done: true };
      },
    };
  },
};
show("counted_args", countArgs(...countingIterable));
show("next_calls", nextCalls);

// apply reads length and indices; it never iterates.
let iterated = 0;
const arrayLike: any = {
  length: 3,
  0: "x",
  1: "y",
  2: "z",
  [Symbol.iterator]() {
    iterated += 1;
    return [][Symbol.iterator]();
  },
};
show("apply_arraylike", firstLast.apply(null, arrayLike));
show("apply_did_not_iterate", iterated);
show("spread_does_iterate", [firstLast(...arrayLike), iterated]);

// `new` and Reflect.construct accept spreads too.
class Triple {
  joined: string;
  constructor(...parts: any[]) { this.joined = parts.join("-"); }
}
show("new_spread", new Triple(...["p", "q", "r"]).joined);
show("new_spread_mixed", new Triple("head", ...["mid"], "tail").joined);
show("reflect_construct", Reflect.construct(Triple, many).joined.length);

// A bound function prepends its own arguments to the spread ones.
const boundSum: any = sumRest.bind(null, 100, 200);
show("bound_then_spread", boundSum(...[1, 2, 3]));
show("bound_then_apply", boundSum.apply(null, [1, 2, 3]));
show("bound_length", boundSum.length);

// Rest collects everything past the fixed parameters into a fresh real array.
function restShape(first: any, ...others: any[]): string {
  return "first=" + String(first) + " rest=" + others.length + " isArray=" + Array.isArray(others);
}
show("rest_shape", restShape(...many));
show("rest_shape_short", restShape(1));
function bothViews(first: any, ...others: any[]): string {
  return arguments.length + "/" + (others.length + 1);
}
show("both_views", bothViews(...many));

show("order_preserved", (function (...values: number[]): boolean {
  for (let i = 0; i < values.length; i++) if (values[i] !== i + 1) return false;
  return values.length === 128;
})(...many));

// Iterating arguments with spread and map is the old idiom made new.
function collect(): string {
  return [...arguments].map((x) => String(x).toUpperCase()).join(",");
}
show("collect", collect("a", 2, true));
show("collect_empty", collect());
