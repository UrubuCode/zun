// Cross-runtime: the FinalizationRegistry API surface and its unregister
// bookkeeping, observable without any collection. register() and unregister()
// accept and refuse specific values, unregister() returns a genuine boolean
// that reports whether a token matched, and tokens are compared by identity.
// Nothing here forces, awaits, or asserts a collection, and no cleanup
// callback is ever expected to run.

function attempt(label: string, fn: () => any): void {
  try {
    console.log(label + "=ok:" + String(fn()));
  } catch (e: any) {
    console.log(label + "=" + e.name);
  }
}

// --- constructor and API shape ---
const fr = new FinalizationRegistry((held: any) => { void held; });
console.log("fr_tag=" + Object.prototype.toString.call(fr));
const frTag: any = Object.getOwnPropertyDescriptor(FinalizationRegistry.prototype, Symbol.toStringTag);
console.log("fr_tag_desc=" + frTag.value + ":" + frTag.writable + ":" + frTag.enumerable + ":" + frTag.configurable);
console.log("fr_ctor_length=" + FinalizationRegistry.length + ":" + FinalizationRegistry.name);
console.log("fr_register_length=" + fr.register.length + ":" + fr.register.name);
console.log("fr_unregister_length=" + fr.unregister.length + ":" + fr.unregister.name);
console.log("fr_own_keys=" + Reflect.ownKeys(fr).length);
console.log("fr_proto_ctor=" + (FinalizationRegistry.prototype.constructor === FinalizationRegistry));
console.log("fr_typeof_ctor=" + typeof FinalizationRegistry);
console.log("fr_instanceof=" + (fr instanceof FinalizationRegistry));
attempt("fr_ctor_no_callback", () => new (FinalizationRegistry as any)());
attempt("fr_ctor_number_callback", () => new (FinalizationRegistry as any)(42));
attempt("fr_ctor_object_callback", () => new (FinalizationRegistry as any)({}));
attempt("fr_ctor_without_new", () => (FinalizationRegistry as any)(() => {}));
console.log("fr_ctor_arrow_typeof=" + typeof new FinalizationRegistry(() => {}));

// --- a subclass keeps the registry slot ---
class Tracked extends FinalizationRegistry<any> {
  label = "tracked";
}
const sub = new Tracked(() => { /* never called synchronously */ });
console.log("fr_subclass_label=" + sub.label + ":" + (sub instanceof FinalizationRegistry));
console.log("fr_subclass_register=" + String(sub.register({ id: "s" }, "h")));

// --- brand checks on the methods ---
const brandTarget: any = { id: "brand" };
attempt("fr_register_on_plain", () => FinalizationRegistry.prototype.register.call({} as any, brandTarget, "h"));
attempt("fr_unregister_on_plain", () => FinalizationRegistry.prototype.unregister.call({} as any, brandTarget));
attempt("fr_register_on_prototype", () => (FinalizationRegistry.prototype as any).register(brandTarget, "h"));

// --- register returns undefined and accepts any held value except the target ---
const a: any = { id: "a" };
const b: any = { id: "b" };
const token: any = { id: "token" };
console.log("fr_register_returns=" + String(fr.register(a, "heldA")));
console.log("fr_register_with_token=" + String(fr.register(b, "heldB", token)));
console.log("fr_register_again=" + String(fr.register(a, "heldA2")));
console.log("fr_register_same_token_twice=" + String(fr.register(a, "heldA3", token)));
console.log("fr_held_undefined=" + String(fr.register(a, undefined)));
console.log("fr_held_number=" + String(fr.register(a, 42)));
console.log("fr_held_other_object=" + String(fr.register(a, b)));
attempt("fr_target_is_held_value", () => fr.register(a, a));
attempt("fr_held_is_token", () => fr.register(b, tokenHeldSelf(), tokenHeldSelf()));
attempt("fr_held_is_registry", () => fr.register(b, fr));
attempt("fr_held_is_symbol", () => fr.register(b, Symbol("h")));
attempt("fr_token_is_target", () => fr.register(b, "held", b));

function tokenHeldSelf(): any { return { self: true }; }

// --- register refuses targets that cannot be held weakly ---
attempt("fr_register_number", () => fr.register(1 as any, "h"));
attempt("fr_register_string", () => fr.register("s" as any, "h"));
attempt("fr_register_boolean", () => fr.register(true as any, "h"));
attempt("fr_register_null", () => fr.register(null as any, "h"));
attempt("fr_register_undefined", () => fr.register(undefined as any, "h"));
attempt("fr_register_bigint", () => fr.register(10n as any, "h"));
attempt("fr_register_registered_symbol", () => fr.register(Symbol.for("fr-key") as any, "h"));
attempt("fr_register_unique_symbol", () => fr.register(Symbol("uniq") as any, "h"));

// --- the unregister token must be holdable: undefined is allowed, primitives are not ---
attempt("fr_token_number", () => fr.register(a, "h", 1 as any));
attempt("fr_token_string", () => fr.register(a, "h", "tok" as any));
console.log("fr_token_undefined=" + String(fr.register(a, "h", undefined)));
attempt("fr_unregister_string", () => fr.unregister("tok" as any));
attempt("fr_unregister_number", () => fr.unregister(1 as any));
attempt("fr_unregister_undefined", () => fr.unregister(undefined as any));
console.log("fr_unregister_unique_symbol=" + fr.unregister(Symbol("t") as any));
attempt("fr_token_registered_symbol", () => fr.register(a, "reg", Symbol.for("fr-token") as any));

// --- unregister returns a boolean that reports whether a cell was removed ---
console.log("fr_unregister_known=" + fr.unregister(token));
console.log("fr_unregister_again=" + fr.unregister(token));
console.log("fr_unregister_unknown=" + fr.unregister({} as any));
console.log("fr_unregister_returns_boolean=" + typeof fr.unregister(token));
console.log("fr_unregister_strict_false=" + (fr.unregister(token) === false));

// --- one token covering several cells clears them all in a single call ---
const c: any = { id: "c" };
const tok1: any = { id: "tok1" };
const tok2: any = { id: "tok2" };
fr.register(a, "a1", tok1);
fr.register(b, "b1", tok1);
fr.register(c, "c1", tok1);
console.log("multi_first=" + fr.unregister(tok1));
console.log("multi_again=" + fr.unregister(tok1));
console.log("multi_third=" + fr.unregister(tok1));
console.log("never_used=" + fr.unregister(tok2));
console.log("fresh_object=" + fr.unregister({ id: "fresh" }));

// --- a cell registered without a token can never be unregistered ---
fr.register(a, "no_token");
console.log("no_token_by_target=" + fr.unregister(a));
console.log("no_token_by_other=" + fr.unregister(tok2));

// --- the target is a legal token, and unregistering by it works ---
fr.register(b, "self_token", b);
console.log("target_as_token=" + fr.unregister(b));
console.log("target_as_token_again=" + fr.unregister(b));

// --- the target of a cell is not a key; the token is ---
fr.register(c, "under_tok2", tok2);
console.log("wrong_key_is_target=" + fr.unregister(c));
console.log("right_key_is_token=" + fr.unregister(tok2));

// --- identity, not structural equality, decides a match ---
const shaped1: any = { same: 1 };
const shaped2: any = { same: 1 };
fr.register(a, "shaped", shaped1);
console.log("lookalike_token=" + fr.unregister(shaped2));
console.log("real_token=" + fr.unregister(shaped1));

// --- a token stays usable after the cell it covered is gone ---
fr.register(a, "again", tok1);
console.log("reused_token=" + fr.unregister(tok1));
console.log("reused_token_again=" + fr.unregister(tok1));

// --- two registries keep separate books over the same token ---
const other = new FinalizationRegistry((held: any) => { void held; });
fr.register(a, "in_fr", tok1);
other.register(a, "in_other", tok1);
console.log("cross_first=" + other.unregister(tok1));
console.log("cross_other_empty=" + other.unregister(tok1));
console.log("cross_original_intact=" + fr.unregister(tok1));

// --- one target may hold many cells under different tokens ---
fr.register(a, "h1", tok1);
fr.register(a, "h2", tok2);
fr.register(a, "h3");
console.log("many_cells_tok1=" + fr.unregister(tok1));
console.log("many_cells_tok2=" + fr.unregister(tok2));
console.log("many_cells_tok1_again=" + fr.unregister(tok1));

// --- a unique symbol works as a token ---
const symTok: any = Symbol("token");
fr.register(a, "sym", symTok);
console.log("symbol_token=" + fr.unregister(symTok));
console.log("symbol_token_again=" + fr.unregister(symTok));

// --- a Proxy token is its own identity, and no trap fires ---
const trapLog: string[] = [];
const rawTok: any = { id: "raw" };
const proxyTok: any = new Proxy(rawTok, {
  get(t: any, k: any, r: any) { trapLog.push("get:" + String(k)); return Reflect.get(t, k, r); },
  has(t: any, k: any) { trapLog.push("has"); return Reflect.has(t, k); },
});
fr.register(a, "proxy_tok", proxyTok);
console.log("proxy_token_raw_misses=" + fr.unregister(rawTok));
console.log("proxy_token_hits=" + fr.unregister(proxyTok));
console.log("proxy_traps=" + JSON.stringify(trapLog.join("|")));

// --- held values and tokens of exotic kinds are accepted ---
const exoticHeld: any[] = [Object.create(null), [1, 2], new Map(), new Uint8Array(2), Math, globalThis];
exoticHeld.forEach((v, i) => fr.register(a, v, { slot: i }));
console.log("exotic_held_count=" + exoticHeld.length);
console.log("exotic_held_register=" + String(fr.register(c, [], tok2)));
console.log("exotic_held_unregister=" + fr.unregister(tok2));
