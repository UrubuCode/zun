// Nullish coalescing, optional chaining (?. ?.() ?.[]), delete through a chain,
// and the logical assignment operators on variables and properties.

type User = { name?: string; tags?: string[]; address?: { city?: string } };
const missing: User | null = null;
const partial: User = {};
const rich: User = { name: "Ada", tags: ["core", "fast"], address: { city: "NYC" } };

// --- ?? keeps every falsy value that is not nullish; || replaces all of them ---
const falsy: any[] = [0, "", false, NaN, null, undefined];
console.log("nullish_map=" + falsy.map((v) => String(v ?? "N")).join("|"));
console.log("or_map=" + falsy.map((v) => String(v || "O")).join("|"));
console.log("nullish_literal=" + (null ?? "fallback"));

// --- optional property, index and nested lookups ---
console.log("null_name=" + (missing?.name ?? "anon"));
console.log("missing_name=" + (partial.name ?? "anon"));
console.log("rich_name=" + (rich.name ?? "anon"));
console.log("tag0=" + (rich.tags?.[0] ?? "none"));
console.log("tag9=" + (rich.tags?.[9] ?? "none"));
console.log("missing_tag=" + (partial.tags?.[0] ?? "none"));
console.log("null_chain=" + (missing?.tags?.[0] ?? "none"));
console.log("chain_mix=" + ((missing?.name ?? "x") + ":" + (rich.tags?.[1] ?? "y")));
console.log("city_lookup=" + (partial.address?.city ?? "unknown") + "|" + (rich.address?.city ?? "unknown"));

// --- optional method and index calls on arrays and records ---
const arr: number[] | null = [1, 2, 3];
const none: number[] | null = null;
console.log("opt_method=" + arr?.join(",") + "|" + none?.join(","));
const data: Record<string, number[]> | null = { keys: [1, 2] };
console.log("opt_index=" + data?.keys?.[0] + "|" + (null as any)?.keys?.[0]);

// --- an optional call keeps the receiver, and a missing callee is just undefined ---
const obj: any = {
  x: 5,
  getSelf() { return this; },
  child: { x: 9, m() { return this.x; } },
};
console.log("this_getself=" + obj.getSelf?.().x);
console.log("this_child=" + obj.child?.m?.());
const detached = obj.child?.m;
console.log("this_call_override=" + detached?.call({ x: 12 }));
console.log("this_absent=" + (null as any)?.m?.());
console.log("optional_method_missing=" + obj.nothere?.());
console.log("parenthesised_keeps_this=" + (obj.child?.m)?.());
const bound: any = { tag: "self", read(): string { return this.tag; } };
console.log("this_binding=" + bound?.read() + "|" + bound?.["read"]());
let nonFn = "none";
try {
  nonFn = String(obj.child?.x());
} catch (e) {
  nonFn = (e as any).constructor.name;
}
console.log("non_function_call=" + nonFn);

// --- a short-circuit skips the whole rest of its chain, including arguments ---
const nil: any = null;
const calls: string[] = [];
function note(name: string): any { calls.push(name); return 0; }
console.log("deep_chain=" + nil?.b.c.d);
console.log("deep_index_call=" + nil?.b[0]().c);
console.log("deep_type=" + typeof nil?.b.c.d);
nil?.b[note("index")];
nil?.b(note("arg"));
nil?.[note("computed")];
nil?.b.c[note("deep_index")].d(note("deep_arg"));
console.log("short_no_eval=" + calls.length);
obj.child[note("live_index")];
console.log("live_eval=" + calls.join(","));

// --- parentheses end the chain: (a?.b).c throws, a?.b.c does not ---
try {
  const v = (nil?.b).c;
  console.log("parens_prop=false:" + v);
} catch (e) {
  console.log("parens_prop=true:" + (e as any).constructor.name);
}
try {
  const v = (nil?.b)[0];
  console.log("parens_index=false:" + v);
} catch (e) {
  console.log("parens_index=true:" + (e as any).constructor.name);
}
console.log("no_parens=" + nil?.b.c + "|" + nil?.b() + "|" + nil?.b[0]);
console.log("parens_live=" + (obj.child).x);
try {
  const v = obj.missing?.deep.value;
  console.log("guard_left=" + v);
} catch (e) {
  console.log("guard_left_threw=" + (e as any).constructor.name);
}
try {
  const v = obj.missing.deep?.value;
  console.log("guard_too_late=false:" + v);
} catch (e) {
  console.log("guard_too_late=true:" + (e as any).constructor.name);
}

// --- short-circuit value is undefined, never null; it composes with ?? and + ---
console.log("is_undefined=" + (nil?.b === undefined) + "|not_null=" + (nil?.b === null));
console.log("with_nullish=" + (nil?.b ?? "fallback") + "|" + (nil?.b + 1) + "|" + `${nil?.b}`);
console.log("sibling_chain=" + (nil?.b) + "/" + obj?.child.x);

// --- delete through an optional chain ---
const killable: any = { drop: 2, keep: 1 };
console.log("delete_short=" + (delete nil?.a) + "|" + (delete nil?.b[note("del")]) + "|" + calls.length);
console.log("delete_live=" + (delete killable?.drop) + "|" + Object.keys(killable).join(","));

// --- logical assignment: ||= assigns when falsy, &&= when truthy, ??= when nullish ---
let la: any = 0;
la ||= 5;
let lb: any = "x";
lb &&= "y";
let lc: any = null;
lc ??= 9;
console.log("logical_vars=" + la + "," + lb + "," + lc);
const lo: any = { v: 0, w: "ok", x: null, z: undefined };
lo.v ||= 2;
lo.w &&= "done";
lo["x"] ??= 3;
lo.z ??= "filled";
console.log("logical_props=" + lo.v + "," + lo.w + "," + lo.x + "," + lo.z);
const rlog: string[] = [];
function rhs(tag: string, val: number): number { rlog.push(tag); return val; }
let la2: any = 1;
la2 &&= rhs("A", 2);
let lb2: any = 0;
lb2 &&= rhs("B", 9);
let lc2: any = null;
lc2 ??= rhs("C", 7);
let ld2: any = 5;
ld2 ??= rhs("D", 99);
let le2: any = 0;
le2 ||= rhs("E", 3);
let lf2: any = 8;
lf2 ||= rhs("F", 100);
console.log("logical_values=" + [la2, lb2, lc2, ld2, le2, lf2].join(",") + "|" + rlog.join(","));
