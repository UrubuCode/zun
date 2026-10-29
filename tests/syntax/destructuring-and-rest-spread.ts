// Destructuring in declarations, parameters, assignments and member targets,
// plus rest elements and array/call spread.

// --- declarations: array and object patterns with picks, defaults and rest ---
const coords = [10, 20, 30, 40];
const [first, second = 0, third, ...rest] = coords;
console.log("arr_pick=" + [first, second, third].join(","));
console.log("arr_rest=" + rest.join(","));

const cfg: any = { host: "localhost", port: 8080, secure: "off", mode: "dev" };
const { host, port, missing = "fallback", ...cfgRest } = cfg;
console.log("obj_pick=" + host + ":" + port + ":" + missing);
console.log("obj_rest_keys=" + Object.keys(cfgRest).join(","));
console.log("obj_rest_vals=" + [cfgRest.secure, cfgRest.mode].join(","));

// a default fires only for undefined; null and 0 are kept
const nd: any = { a: { x: 0 }, b: undefined, c: null };
const { a: { x = 9, y = 8 }, b = 7, c = 6 } = nd;
console.log("defaults_undefined_only=" + x + "," + y + "," + b + "," + c);

// a rename with a default, and a nested default that comes from a missing object
const { foo: bar = 42 } = {} as any;
console.log("rename_default=" + bar);
const { x: { y: deepY = 5 } = {} } = { x: {} } as any;
console.log("nested_default=" + deepY);

// elision in the pattern consumes a position without naming it
const input = [10, 20, 30, 40, 50];
const [e1, , e3, ...eTail] = input;
console.log("elision_rest=" + e1 + "," + e3 + "," + eTail.join(",") + "|len=" + input.length);
const [, second2 = "S", , fourth = "F"] = [10, 20, 30] as any[];
console.log("pattern_elision=" + second2 + "," + fourth);

// holes in a source array read as undefined, so the default fires
const holed = [1, , 3, 4] as any[];
const [h1, h2 = h1 + 10, ...hTail] = holed;
console.log("hole_pattern=" + h1 + "," + h2 + "," + hTail.join("|") + ",len=" + hTail.length);

// computed keys in a pattern, with a default that reads an earlier binding
const key = "left";
const src: any = { left: 3, right: undefined, extra: 9, nested: { value: 5 } };
const { [key]: left, right = left * 4, nested: { value: nv = 0 }, ...srcRest } = src;
console.log("computed_key=" + left + "|" + right + "|" + nv);
console.log("computed_rest=" + Object.keys(srcRest).sort().join(",") + ":" + JSON.stringify(srcRest));

// --- parameter patterns ---
function greet({ name = "world", times = 1 }: { name?: string; times?: number }): string {
  let out = "";
  for (let i = 0; i < times; i++) out += "hello " + name + ";";
  return out;
}
console.log("param_object=" + greet({}) + "|" + greet({ name: "rts", times: 2 }));
function pair([p, q = p * 2]: number[]): string { return p + ":" + q; }
console.log("param_array=" + pair([3]) + "|" + pair([3, 1]));

// --- the iterator is closed when a default initialiser throws ---
const dlog: string[] = [];
const iter: any = {
  [Symbol.iterator]() {
    let i = 0;
    return {
      next() { dlog.push("next" + i); return { value: i++ === 0 ? undefined : 2, done: false }; },
      return() { dlog.push("return"); return { done: true }; },
    };
  },
};
try {
  const [dx = (() => { throw new RangeError("default"); })(), dy] = iter;
  console.log("unreached=" + dx + dy);
} catch (e) {
  console.log("default_threw=" + (e as any).message);
}
console.log("default_throw_log=" + dlog.join(","));

// --- rest parameters are fresh, mutable arrays ---
function collect(head: number, ...tail: number[]): number[] {
  tail.push(head);
  return tail;
}
const srcNums = [1, 2, 3];
const collected = collect(...srcNums);
console.log("rest_independent=" + srcNums.join(",") + "|" + collected.join(",") + "|" + (srcNums === collected));
console.log("rest_length=" + collect(0).length + "|" + collect(0, 1, 2).length);

// --- spread into calls and array literals ---
function sum3(a: number, b: number, c: number): number { return a + b + c; }
console.log("spread_call=" + sum3(...[4, 5, 6]));
const clone = [...coords.slice(0, 2), 99, ...coords.slice(2)];
console.log("spread_clone=" + clone.join(","));
console.log("spread_string=" + [..."hello"].length + "|" + [..."ab", ..."cd"].join(""));

// --- assignment patterns write existing bindings ---
let ea = 0, eb = 0;
let erest: number[] = [];
[ea, eb, ...erest] = [1, 2, 3, 4];
console.log("assign_rest=" + ea + "," + eb + "," + erest.join(","));
({ a: ea, b: eb } = { a: 7, b: 8 } as any);
console.log("assign_obj=" + ea + "," + eb);
let da = 0, db = 0;
const src2 = { da: 1, db: 2 };
const daResult = ({ da, db } = src2);
console.log("obj_assign_value=" + da + "," + db + "|" + (daResult === src2));
let sx = "left", sy = "right";
[sx, sy] = [sy, sx];
let r1 = 1, r2 = 2, r3 = 3;
[r1, r2, r3] = [r3, r1, r2];
console.log("swap_rotate=" + sx + "," + sy + "|" + r1 + "," + r2 + "," + r3);

// --- assignment patterns whose targets are member expressions ---
const mt: any = {};
const mholder: any = { arr: [0, 0, 0] };
const mlog: string[] = [];
let midx = 0;
function nextKey(): number { mlog.push("key" + midx); return midx++; }
[mt.a, mholder.arr[nextKey()], mt.b] = [1, 2, 3];
console.log("member_targets=" + mt.a + "," + mholder.arr.join("") + "," + mt.b + "|" + mlog.join(","));

const slog: string[] = [];
const setters: any = {
  _x: 0,
  _y: 0,
  set x(v: number) { slog.push("set_x=" + v); this._x = v; },
  set y(v: number) { slog.push("set_y=" + v); this._y = v; },
};
[setters.x, setters.y] = [11, 22];
console.log("setter_order=" + slog.join(",") + "|" + setters._x + "/" + setters._y);

const getterSrc: any = {
  get first() { slog.push("get_first"); return 1; },
  get second() { slog.push("get_second"); return 2; },
};
const into: any = {};
slog.length = 0;
({ first: into.a, second: into.b } = getterSrc);
console.log("getter_order=" + slog.join(",") + "|" + into.a + into.b);

const objRest: any = {};
({ keep: objRest.kept, ...objRest.others } = { keep: 1, a: 2, b: 3 } as any);
console.log("obj_rest_member=" + objRest.kept + "|" + JSON.stringify(objRest.others));
const restTarget: any = {};
[restTarget.head, ...restTarget.tail] = [1, 2, 3, 4];
console.log("rest_member=" + restTarget.head + "|" + restTarget.tail.join("-") + "|" + Array.isArray(restTarget.tail));
const nest: any = { out: {} };
[[nest.out.p], { q: nest.out.q }] = [[5], { q: 6 }] as any;
console.log("nested_member=" + nest.out.p + "," + nest.out.q);
const fromSet: any = {};
[fromSet.x, fromSet.y] = new Set(["s1", "s2", "s3"]);
const fromStr: any = {};
[fromStr.c0, fromStr.c1] = "hi";
console.log("iterable_sources=" + fromSet.x + "," + fromSet.y + "|" + fromStr.c0 + fromStr.c1);
const rhsArr = [1, 2];
const outT: any = {};
const assignedValue = ([outT.p, outT.q] = rhsArr);
console.log("assign_value_is_rhs=" + (assignedValue === rhsArr) + "|" + outT.p + outT.q);
