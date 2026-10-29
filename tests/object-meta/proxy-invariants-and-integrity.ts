export {};
// Proxy invariants and integrity: every trap answer that is checked against the
// target's own slots (non-configurable, non-writable, non-extensible), the
// integrity operations that are spelled out in terms of those traps, and what
// revocation does to every operation, including revocation issued from inside a
// trap while the same operation is still running.

function show(label: string, value: unknown): void {
  console.log(label + "=" + String(value));
}

function attempt(label: string, fn: () => unknown): void {
  try {
    show(label, fn());
  } catch (e) {
    console.log(label + "=throw:" + (e as Error).constructor.name);
  }
}

// ---------------------------------------------------------------------------
// 1. get invariant: a non-configurable, non-writable data slot is fixed
// ---------------------------------------------------------------------------
const gt: any = {};
Object.defineProperty(gt, "frozen", { value: "REAL", writable: false, configurable: false, enumerable: true });
Object.defineProperty(gt, "roAcc", { get: undefined, set: undefined, configurable: false, enumerable: true });
Object.defineProperty(gt, "writable", { value: "W", writable: true, configurable: false, enumerable: true });
gt.loose = "L";

attempt("get_fixed_lie", () => new Proxy(gt, { get() { return "FAKE"; } }).frozen);
attempt("get_fixed_same", () => new Proxy(gt, { get() { return "REAL"; } }).frozen);
attempt("get_writable_nc_lie", () => new Proxy(gt, { get() { return "FAKE"; } }).writable);
attempt("get_configurable_lie", () => new Proxy(gt, { get() { return "FAKE"; } }).loose);
attempt("get_getterless_acc_lie", () => new Proxy(gt, { get() { return "FAKE"; } }).roAcc);
attempt("get_getterless_acc_undef", () => new Proxy(gt, { get() { return undefined; } }).roAcc);

// ---------------------------------------------------------------------------
// 2. has invariant: cannot hide a non-configurable key, or any key of a sealed target
// ---------------------------------------------------------------------------
attempt("has_hide_nonconfig", () => "frozen" in new Proxy(gt, { has() { return false; } }));
attempt("has_hide_configurable", () => "loose" in new Proxy(gt, { has() { return false; } }));
const shutForHas: any = Object.preventExtensions({ k: 1 });
attempt("has_hide_on_nonextensible", () => "k" in new Proxy(shutForHas, { has() { return false; } }));
attempt("has_invent_key", () => "nope" in new Proxy(gt, { has() { return true; } }));

// ---------------------------------------------------------------------------
// 3. getOwnPropertyDescriptor invariant
// ---------------------------------------------------------------------------
attempt("gopd_hide_nonconfig", () => Object.getOwnPropertyDescriptor(new Proxy(gt, { getOwnPropertyDescriptor() { return undefined; } }), "frozen"));
attempt("gopd_hide_configurable", () => Object.getOwnPropertyDescriptor(new Proxy(gt, { getOwnPropertyDescriptor() { return undefined; } }), "loose"));
attempt("gopd_claim_configurable_for_nonconfig", () => {
  const px: any = new Proxy(gt, {
    getOwnPropertyDescriptor() { return { value: "REAL", configurable: true, writable: false, enumerable: true }; },
  });
  return "c=" + (Object.getOwnPropertyDescriptor(px, "frozen") as any).configurable;
});
attempt("gopd_invent_nonconfig", () => {
  const px: any = new Proxy({} as any, {
    getOwnPropertyDescriptor() { return { value: 1, configurable: false, writable: true, enumerable: true }; },
  });
  return "c=" + (Object.getOwnPropertyDescriptor(px, "any") as any).configurable;
});
attempt("gopd_invent_configurable", () => {
  const px: any = new Proxy({} as any, {
    getOwnPropertyDescriptor() { return { value: 1, configurable: true }; },
  });
  const d = Object.getOwnPropertyDescriptor(px, "any") as any;
  return "v=" + d.value + ",w=" + d.writable + ",e=" + d.enumerable + ",c=" + d.configurable;
});
attempt("gopd_non_object_result", () => Object.getOwnPropertyDescriptor(new Proxy({} as any, { getOwnPropertyDescriptor() { return 1 as any; } }), "x"));

// ---------------------------------------------------------------------------
// 4. deleteProperty invariant: a non-configurable own key cannot be deleted
// ---------------------------------------------------------------------------
attempt("delete_lie_nonconfig", () => Reflect.deleteProperty(new Proxy(gt, { deleteProperty() { return true; } }), "frozen"));
attempt("delete_lie_strict", () => { delete (new Proxy(gt, { deleteProperty() { return true; } }) as any).frozen; return "deleted"; });
attempt("delete_lie_configurable", () => Reflect.deleteProperty(new Proxy({ z: 1 } as any, { deleteProperty() { return true; } }), "z"));
const nonConfigTarget: any = {};
Object.defineProperty(nonConfigTarget, "fixed", { value: 5, configurable: false });
attempt("delete_nonconfig_report_true", () => Reflect.deleteProperty(new Proxy(nonConfigTarget, { deleteProperty() { return true; } }), "fixed"));
show("delete_nonconfig_target_kept", Object.hasOwn(nonConfigTarget, "fixed"));

// ---------------------------------------------------------------------------
// 5. set invariant: a lie is legal over a plain slot, fatal over a fixed one
// ---------------------------------------------------------------------------
const sfTarget: any = {};
Object.defineProperty(sfTarget, "fixed", { value: "REAL", writable: false, enumerable: true, configurable: false });
Object.defineProperty(sfTarget, "ncWritable", { value: "W", writable: true, enumerable: true, configurable: false });
sfTarget.loose = "L";
const sfLiar: any = new Proxy(sfTarget, { set() { return true; } });
attempt("set_lie_fixed", () => Reflect.set(sfLiar, "fixed", "OTHER"));
show("set_lie_fixed_same", Reflect.set(sfLiar, "fixed", "REAL"));
show("set_lie_nc_writable", Reflect.set(sfLiar, "ncWritable", "OTHER"));
show("set_lie_loose", Reflect.set(sfLiar, "loose", "OTHER"));
show("set_lie_fixed_target", sfTarget.fixed);

const accSetterless: any = {};
Object.defineProperty(accSetterless, "readOnly", { get() { return 1; }, configurable: false, enumerable: true });
Object.defineProperty(accSetterless, "confReadOnly", { get() { return 1; }, configurable: true, enumerable: true });
const accLiar: any = new Proxy(accSetterless, { set() { return true; } });
attempt("set_lie_setterless_nonconfig", () => Reflect.set(accLiar, "readOnly", 2));
show("set_lie_setterless_configurable", Reflect.set(accLiar, "confReadOnly", 2));

// a refused set is false through Reflect and a TypeError through the operators
const refusedTarget: any = { a: 1 };
const refuses: any = new Proxy(refusedTarget, { set() { return false; } });
show("set_refused_reflect", Reflect.set(refuses, "a", 2));
show("set_refused_target", refusedTarget.a);
attempt("set_refused_assign", () => { refuses.a = 3; return "ok"; });
attempt("set_refused_object_assign", () => { Object.assign(refuses, { a: 4 }); return "ok"; });
const truthySet: any = new Proxy({ a: 1 }, { set() { return "yes" as any; } });
show("set_truthy_report", Reflect.set(truthySet, "a", 2));
const zeroSet: any = new Proxy({ a: 1 }, { set() { return 0 as any; } });
show("set_zero_report", Reflect.set(zeroSet, "a", 2));
const undefSet: any = new Proxy({ a: 1 }, { set() { /* undefined */ } });
show("set_undefined_report", Reflect.set(undefSet, "a", 2));

// ---------------------------------------------------------------------------
// 6. defineProperty invariants: no new key on a sealed target, a non-configurable
//    definition needs a non-configurable target slot, and a configurable target
//    slot cannot be upgraded to non-configurable through the trap
// ---------------------------------------------------------------------------
const shut: any = Object.preventExtensions({ old: 1 });
const shutProxy: any = new Proxy(shut, { defineProperty() { return true; } });
attempt("define_new_on_nonextensible", () => Reflect.defineProperty(shutProxy, "brand", { value: 1, configurable: true }));
show("define_existing_on_nonextensible", Reflect.defineProperty(shutProxy, "old", { value: 9, configurable: true }));
show("define_nonextensible_target", shut.old);

const emptyTarget: any = {};
const inventor: any = new Proxy(emptyTarget, { defineProperty() { return true; } });
attempt("define_invent_nonconfig", () => Reflect.defineProperty(inventor, "ghost", { value: 1, configurable: false }));
show("define_invent_configurable", Reflect.defineProperty(inventor, "ghost", { value: 1, configurable: true }));
show("define_invent_target_count", Object.getOwnPropertyNames(emptyTarget).length);

const looseTarget: any = { loose: 1 };
const upgrader: any = new Proxy(looseTarget, { defineProperty() { return true; } });
attempt("define_upgrade_configurable_to_nonconfig", () => Reflect.defineProperty(upgrader, "loose", { value: 1, configurable: false }));

const fixedTarget: any = {};
Object.defineProperty(fixedTarget, "fixed", { value: 1, writable: false, enumerable: true, configurable: false });
const honest: any = new Proxy(fixedTarget, { defineProperty() { return true; } });
show("define_honest_nonconfig", Reflect.defineProperty(honest, "fixed", { value: 1, writable: false, enumerable: true, configurable: false }));
attempt("define_incompatible_nonconfig", () => Reflect.defineProperty(honest, "fixed", { value: 2, configurable: false }));
show("define_fixed_value", fixedTarget.fixed);

// ---------------------------------------------------------------------------
// 7. ownKeys invariants: no duplicates, keys only, every non-configurable key
//    reported, and a non-extensible target reported EXACTLY
// ---------------------------------------------------------------------------
const freeKeys = new Proxy({ a: 1, b: 2 }, { ownKeys() { return ["b", "invented", "a"]; } });
attempt("ownKeys_free_invent", () => Reflect.ownKeys(freeKeys).join("|"));
attempt("ownKeys_free_names", () => Object.getOwnPropertyNames(freeKeys).join("|"));
attempt("ownKeys_free_keys", () => Object.keys(freeKeys).join("|"));
attempt("ownKeys_duplicate", () => Reflect.ownKeys(new Proxy({ a: 1 }, { ownKeys() { return ["a", "a"]; } })).join("|"));
attempt("ownKeys_non_key_entry", () => Reflect.ownKeys(new Proxy({}, { ownKeys() { return [1 as any]; } })).join("|"));
attempt("ownKeys_non_list_result", () => Reflect.ownKeys(new Proxy({}, { ownKeys() { return "ab" as any; } })).join("|"));

const ncHolder: any = {};
Object.defineProperty(ncHolder, "fixed", { value: 1, configurable: false, enumerable: true });
ncHolder.loose = 2;
attempt("ownKeys_hide_nonconfig", () => Reflect.ownKeys(new Proxy(ncHolder, { ownKeys() { return ["loose"]; } })).join("|"));
attempt("ownKeys_report_nonconfig", () => Reflect.ownKeys(new Proxy(ncHolder, { ownKeys() { return ["fixed"]; } })).join("|"));

const sealedKeys: any = { x: 1, y: 2 };
Object.preventExtensions(sealedKeys);
attempt("ownKeys_nonext_extra", () => Reflect.ownKeys(new Proxy(sealedKeys, { ownKeys() { return ["x", "y", "z"]; } })).join("|"));
attempt("ownKeys_nonext_missing", () => Reflect.ownKeys(new Proxy(sealedKeys, { ownKeys() { return ["x"]; } })).join("|"));
attempt("ownKeys_nonext_reorder", () => Reflect.ownKeys(new Proxy(sealedKeys, { ownKeys() { return ["y", "x"]; } })).join("|"));

const symA = Symbol("s1");
const symB = Symbol("s2");
const symKeys = new Proxy({}, { ownKeys() { return ["a", symB, "b", symA]; } });
attempt("ownKeys_symbols_all", () => Reflect.ownKeys(symKeys).map(String).join("|"));
attempt("ownKeys_symbols_names", () => Object.getOwnPropertyNames(symKeys).join("|"));
attempt("ownKeys_symbols_only", () => Object.getOwnPropertySymbols(symKeys).map(String).join("|"));

const countedKeys = new Proxy({ a: 1, b: 2 }, {
  ownKeys(t) { countedCalls++; return Reflect.ownKeys(t); },
});
let countedCalls = 0;
Object.keys(countedKeys);
show("ownKeys_calls_keys", countedCalls);
countedCalls = 0;
JSON.stringify(countedKeys);
show("ownKeys_calls_json", countedCalls);

// ---------------------------------------------------------------------------
// 8. Extensibility traps: isExtensible repeats the target, preventExtensions is
//    only honest once the target is really sealed
// ---------------------------------------------------------------------------
const plainP: any = new Proxy({}, {});
show("ext_forward_before", Reflect.isExtensible(plainP));
show("ext_forward_prevent", Reflect.preventExtensions(plainP));
show("ext_forward_after", Reflect.isExtensible(plainP));

const openTarget: any = {};
show("isext_agree_true", Reflect.isExtensible(new Proxy(openTarget, { isExtensible() { return true; } })));
show("isext_agree_truthy", Reflect.isExtensible(new Proxy(openTarget, { isExtensible() { return 1 as any; } })));
show("isext_agree_string", Reflect.isExtensible(new Proxy(openTarget, { isExtensible() { return "yes" as any; } })));
attempt("isext_lie_false", () => Reflect.isExtensible(new Proxy(openTarget, { isExtensible() { return false; } })));
attempt("isext_lie_zero", () => Reflect.isExtensible(new Proxy(openTarget, { isExtensible() { return 0 as any; } })));
attempt("isext_lie_undefined", () => Reflect.isExtensible(new Proxy(openTarget, { isExtensible() { return undefined as any; } })));

const shutTarget: any = Object.preventExtensions({});
show("isext_shut_agree", Reflect.isExtensible(new Proxy(shutTarget, { isExtensible() { return false; } })));
attempt("isext_shut_lie", () => Reflect.isExtensible(new Proxy(shutTarget, { isExtensible() { return true; } })));
show("isext_object_agree", Object.isExtensible(new Proxy(shutTarget, {})));
attempt("isext_object_lie", () => Object.isExtensible(new Proxy(shutTarget, { isExtensible() { return true; } })));

const stillOpen: any = {};
attempt("prevent_claims_but_open", () => Reflect.preventExtensions(new Proxy(stillOpen, { preventExtensions() { return true; } })));
show("prevent_still_open", Object.isExtensible(stillOpen));

const honestTarget: any = {};
show("prevent_honest", Reflect.preventExtensions(new Proxy(honestTarget, {
  preventExtensions(t) { Object.preventExtensions(t); return true; },
})));
show("prevent_honest_target", Object.isExtensible(honestTarget));

const refusingPrevent: any = new Proxy({}, { preventExtensions() { return false; } });
show("prevent_refused_reflect", Reflect.preventExtensions(refusingPrevent));
attempt("prevent_refused_object", () => { Object.preventExtensions(refusingPrevent); return "ok"; });

const alreadyShut: any = Object.preventExtensions({});
show("prevent_already_shut_true", Reflect.preventExtensions(new Proxy(alreadyShut, { preventExtensions() { return true; } })));

const returnedTarget: any = {};
const returnedProxy: any = new Proxy(returnedTarget, {});
show("prevent_returns_proxy", Object.preventExtensions(returnedProxy) === returnedProxy);
show("prevent_returns_not_target", Object.preventExtensions(returnedProxy) === returnedTarget);

// after preventExtensions, adding through the proxy is refused even with a set trap
const closed: any = Object.preventExtensions({ a: 1 });
const closedProxy: any = new Proxy(closed, { set(t, k, v, r) { return Reflect.set(t, k, v, r); } });
show("closed_add_refused", Reflect.set(closedProxy, "b", 1));
show("closed_update_ok", Reflect.set(closedProxy, "a", 2) + ",v=" + closed.a);
show("closed_define_refused", Reflect.defineProperty(closedProxy, "c", { value: 1, configurable: true }));
show("closed_keys", Object.keys(closed).join("|"));

// ---------------------------------------------------------------------------
// 9. Integrity levels (freeze / seal / isFrozen / isSealed) over a proxy: they are
//    not traps of their own, they are spelled out in terms of preventExtensions,
//    ownKeys, getOwnPropertyDescriptor and defineProperty
// ---------------------------------------------------------------------------
const integrityTraps: string[] = [];
const sniffIntegrity: any = new Proxy({ a: 1 }, new Proxy({} as any, {
  get(_t, k) { integrityTraps.push(String(k)); return undefined; },
}));
Object.freeze(sniffIntegrity);
show("integrity_lookups", integrityTraps.join("|"));
show("integrity_has_freeze_trap", integrityTraps.indexOf("freeze") >= 0);
show("integrity_has_isFrozen_trap", integrityTraps.indexOf("isFrozen") >= 0);

const applied: string[] = [];
const halfTarget: any = { a: 1, b: 2, c: 3 };
const half: any = new Proxy(halfTarget, {
  defineProperty(t, k, d) {
    if (k === "b") return false;
    applied.push(String(k));
    return Reflect.defineProperty(t, k, d);
  },
});
attempt("freeze_refused_midway", () => { Object.freeze(half); return "ok"; });
show("freeze_applied", applied.join("|"));
show("freeze_target_a_writable", (Object.getOwnPropertyDescriptor(halfTarget, "a") as any).writable);
show("freeze_target_b_writable", (Object.getOwnPropertyDescriptor(halfTarget, "b") as any).writable);
show("freeze_target_c_writable", (Object.getOwnPropertyDescriptor(halfTarget, "c") as any).writable);
show("freeze_target_extensible", Object.isExtensible(halfTarget));

// isFrozen answers from the handler's descriptors, but the gopd invariant refuses
// to let it be fooled about a configurable target property
const lyingTarget: any = { x: 1 };
Object.preventExtensions(lyingTarget);
const lyingHandler: any = {
  isExtensible() { return false; },
  preventExtensions(t: any) { Object.preventExtensions(t); return true; },
  getOwnPropertyDescriptor() { return { value: 1, writable: false, enumerable: true, configurable: false }; },
  ownKeys() { return ["x"]; },
};
const lying: any = new Proxy(lyingTarget, lyingHandler);
attempt("lying_isFrozen", () => Object.isFrozen(lying));
attempt("lying_isSealed", () => Object.isSealed(lying));
show("lying_target_really_frozen", Object.isFrozen(lyingTarget));
show("lying_target_x_writable", (Object.getOwnPropertyDescriptor(lyingTarget, "x") as any).writable);

// once the target genuinely matches the handler's claims, the answer is true
const honestFrozen: any = {};
Object.defineProperty(honestFrozen, "x", { value: 1, writable: false, enumerable: true, configurable: false });
Object.preventExtensions(honestFrozen);
const honestHandled: any = new Proxy(honestFrozen, lyingHandler);
show("honest_isFrozen", Object.isFrozen(honestHandled));
show("honest_isSealed", Object.isSealed(honestHandled));

const frozenUnder: any = Object.freeze({ y: 1 });
const looseClaim: any = new Proxy(frozenUnder, {
  getOwnPropertyDescriptor() { return { value: 1, writable: true, enumerable: true, configurable: true }; },
});
attempt("loose_isFrozen_on_frozen_target", () => Object.isFrozen(looseClaim));
show("plain_frozen_proxy", Object.isFrozen(new Proxy(Object.freeze({ k: 1 }), {})) + "," + Object.isSealed(new Proxy(Object.freeze({ k: 1 }), {})));
show("plain_frozen_proxy_extensible", Object.isExtensible(new Proxy(Object.freeze({ k: 1 }), {})));

const emptyLog: string[] = [];
const emptySealed: any = new Proxy({}, {
  ownKeys(t) { emptyLog.push("ownKeys"); return Reflect.ownKeys(t); },
  preventExtensions(t) { emptyLog.push("preventExtensions"); Object.preventExtensions(t); return true; },
  defineProperty(t, k, d) { emptyLog.push("define:" + String(k)); return Reflect.defineProperty(t, k, d); },
});
Object.seal(emptySealed);
show("seal_empty_trace", emptyLog.join("|"));
show("seal_empty_isSealed", Object.isSealed(emptySealed));

const fakePrevent: any = new Proxy({ z: 1 }, { preventExtensions() { return true; } });
attempt("fake_prevent_throws", () => Reflect.preventExtensions(fakePrevent));

// ---------------------------------------------------------------------------
// 10. Getting the prototype over a sealed target: the trap must report the real one
// ---------------------------------------------------------------------------
const protoBase = { tag: "base" };
const fixedProto: any = Object.create(protoBase);
Object.preventExtensions(fixedProto);
attempt("setproto_sealed_other", () => Reflect.setPrototypeOf(new Proxy(fixedProto, { setPrototypeOf() { return true; } }), { tag: "other" }));
attempt("setproto_sealed_same", () => Reflect.setPrototypeOf(new Proxy(fixedProto, { setPrototypeOf() { return true; } }), protoBase));
attempt("getproto_sealed_null", () => Object.getPrototypeOf(new Proxy(fixedProto, { getPrototypeOf() { return null; } })));
attempt("getproto_sealed_true", () => Object.getPrototypeOf(new Proxy(fixedProto, { getPrototypeOf() { return protoBase; } })) === protoBase);
attempt("getproto_sealed_refuses_set_object", () => { Object.setPrototypeOf(new Proxy(fixedProto, { setPrototypeOf() { return false; } }), protoBase); return "ok"; });

// ---------------------------------------------------------------------------
// 11. Revocation: what survives, what throws, and that revoke() is idempotent
// ---------------------------------------------------------------------------
const objRev = Proxy.revocable({ a: 1 }, {});
const arrRev = Proxy.revocable([1, 2, 3], {});
const fnRev = Proxy.revocable(function f(x: number) { return x; }, {});

show("revoke_kind", typeof objRev.revoke + ",len=" + (objRev.revoke as any).length);
const revP: any = objRev.proxy;
const revArr: any = arrRev.proxy;
const revFn: any = fnRev.proxy;

show("rev_before_get", revP.a);
show("rev_before_isArray", Array.isArray(revArr));
show("rev_before_tag", Object.prototype.toString.call(revArr));
show("rev_before_call", revFn(5));

const revChild: any = Object.create(revP);
show("rev_before_child", revChild.a);

show("revoke_returns", String(objRev.revoke()));
show("revoke_again", String(objRev.revoke()));
arrRev.revoke();
fnRev.revoke();

show("rev_after_typeof_obj", typeof revP);
show("rev_after_typeof_fn", typeof revFn);
show("rev_after_typeof_arr", typeof revArr);
show("rev_after_identity", revP === objRev.proxy);

attempt("rev_get", () => revP.a);
attempt("rev_set", () => Reflect.set(revP, "a", 2));
attempt("rev_has", () => "a" in revP);
attempt("rev_delete", () => Reflect.deleteProperty(revP, "a"));
attempt("rev_ownKeys", () => Reflect.ownKeys(revP).join("|"));
attempt("rev_keys", () => Object.keys(revP).join("|"));
attempt("rev_gopd", () => Object.getOwnPropertyDescriptor(revP, "a"));
attempt("rev_define", () => Reflect.defineProperty(revP, "b", { value: 1 }));
attempt("rev_getproto", () => Reflect.getPrototypeOf(revP));
attempt("rev_setproto", () => Reflect.setPrototypeOf(revP, null));
attempt("rev_isExtensible", () => Reflect.isExtensible(revP));
attempt("rev_preventExtensions", () => Reflect.preventExtensions(revP));
attempt("rev_json", () => JSON.stringify(revP));
attempt("rev_spread", () => Object.keys({ ...revP }).join("|"));
attempt("rev_forin", () => { let n = 0; for (const _k in revP) { n++; } return n; });
attempt("rev_string", () => String(revP));
attempt("rev_isArray", () => Array.isArray(revArr));
attempt("rev_tag", () => Object.prototype.toString.call(revArr));
attempt("rev_call", () => revFn(5));
attempt("rev_construct", () => typeof new revFn(1));
attempt("rev_instanceof_rhs", () => ({} instanceof revFn));
attempt("rev_instanceof_lhs", () => revP instanceof Object);
attempt("rev_via_child", () => revChild.a);
attempt("rev_freeze", () => { Object.freeze(revP); return "ok"; });

// a revoked proxy is still a legal TARGET; the failure comes at the first operation
const wrapped: any = new Proxy(revP, {});
show("rev_wrap_built", typeof wrapped);
attempt("rev_wrap_get", () => wrapped.a);

// and a legal HANDLER, with the same delayed failure
const handlerRev = Proxy.revocable({ get() { return "h"; } }, {});
const usesHandler: any = new Proxy({ a: 1 }, handlerRev.proxy as any);
show("rev_handler_before", usesHandler.a);
handlerRev.revoke();
attempt("rev_handler_after", () => usesHandler.a);

// revocation from INSIDE a trap: the trap still returns, but the operation's next
// internal step needs the dead proxy and throws
const single = Proxy.revocable({ a: 1 }, {
  get(t, k, r) { single.revoke(); return "trapped:" + Reflect.get(t, k, r); },
});
attempt("revoke_in_get", () => (single.proxy as any).a);
attempt("revoke_in_get_again", () => (single.proxy as any).a);

const duringKeys = Proxy.revocable({ a: 1, b: 2 }, {
  ownKeys(t) { duringKeys.revoke(); return Reflect.ownKeys(t); },
});
attempt("revoke_in_ownKeys_keys", () => Object.keys(duringKeys.proxy).join("|"));

const duringOwnKeys = Proxy.revocable({ a: 1, b: 2 }, {
  ownKeys(t) { duringOwnKeys.revoke(); return Reflect.ownKeys(t); },
});
attempt("revoke_in_ownKeys_reflect", () => Reflect.ownKeys(duringOwnKeys.proxy).join("|"));

const midOrder: string[] = [];
const midway = Proxy.revocable({ a: 1, b: 2, c: 3 }, {
  ownKeys(t) { midOrder.push("ownKeys"); return Reflect.ownKeys(t); },
  getOwnPropertyDescriptor(t, k) {
    midOrder.push("gopd:" + String(k));
    if (k === "b") midway.revoke();
    return Reflect.getOwnPropertyDescriptor(t, k);
  },
});
attempt("revoke_midway_keys", () => Object.keys(midway.proxy).join("|"));
show("revoke_midway_order", midOrder.join(","));

const spreadRev = Proxy.revocable({ a: 1, b: 2 }, {
  getOwnPropertyDescriptor(t, k) { if (k === "b") spreadRev.revoke(); return Reflect.getOwnPropertyDescriptor(t, k); },
});
attempt("revoke_midway_spread", () => Object.keys({ ...(spreadRev.proxy as any) }).join("|"));

const jsonRev = Proxy.revocable({ a: 1, b: 2 }, {
  get(t, k, r) { if (k === "b") jsonRev.revoke(); return Reflect.get(t, k, r); },
});
attempt("revoke_midway_json", () => JSON.stringify(jsonRev.proxy));

const selfRead = Proxy.revocable({ a: 1, other: 2 }, {
  get(t, k, r) {
    if (k === "a") {
      selfRead.revoke();
      return "self:" + (selfRead.proxy as any).other;
    }
    return Reflect.get(t, k, r);
  },
});
attempt("revoke_trap_reads_own_proxy", () => (selfRead.proxy as any).a);

const callRev = Proxy.revocable(function (x: number) { return x * 2; }, {
  apply(t, thisArg, args) { callRev.revoke(); return Reflect.apply(t as any, thisArg, args) + 1; },
});
attempt("revoke_in_apply", () => (callRev.proxy as any)(10));
attempt("revoke_in_apply_again", () => (callRev.proxy as any)(10));

const newRev = Proxy.revocable(class { v = 7; }, {
  construct(t, args, nt) { newRev.revoke(); return Reflect.construct(t as any, args, nt); },
});
attempt("revoke_in_construct", () => new (newRev.proxy as any)().v);

const hasRev = Proxy.revocable({ a: 1 }, { has(t, k) { hasRev.revoke(); return Reflect.has(t, k); } });
attempt("revoke_in_has", () => "a" in (hasRev.proxy as any));
attempt("revoke_in_has_again", () => "a" in (hasRev.proxy as any));

const protoTrapRev = Proxy.revocable({}, { getPrototypeOf(t) { protoTrapRev.revoke(); return Reflect.getPrototypeOf(t); } });
attempt("revoke_in_getProto", () => protoTrapRev.proxy instanceof Object);
attempt("revoke_in_getProto_again", () => protoTrapRev.proxy instanceof Object);

// revocation inside the defineProperty trap of a freeze: preventExtensions has
// already run, so the target is sealed even though the operation dies
const freezeTarget: any = { a: 1, b: 2 };
const freezeRev = Proxy.revocable(freezeTarget, {
  defineProperty(t, k, d) { if (k === "b") freezeRev.revoke(); return Reflect.defineProperty(t, k, d); },
});
attempt("revoke_in_freeze", () => { Object.freeze(freezeRev.proxy); return "ok"; });
show("revoke_freeze_target_extensible", Object.isExtensible(freezeTarget));
show("revoke_freeze_target_a_writable", (Object.getOwnPropertyDescriptor(freezeTarget, "a") as any).writable);
show("revoke_freeze_target_b_writable", (Object.getOwnPropertyDescriptor(freezeTarget, "b") as any).writable);

// revoking a proxy used as a PROTOTYPE part-way through a chain walk
const protoRev = Proxy.revocable({ inherited: "P" }, {
  get(t, k, r) { protoRev.revoke(); return Reflect.get(t, k, r); },
});
const kid: any = Object.create(protoRev.proxy);
kid.own = "O";
show("revoke_proto_kid_own", kid.own);
attempt("revoke_proto_kid_inherited", () => kid.inherited);
attempt("revoke_proto_kid_inherited_again", () => kid.inherited);
show("revoke_proto_kid_own_still", kid.own);

// revoking the MIDDLE proxy poisons every proxy stacked outside it
const mid = Proxy.revocable({ q: 1 }, {});
const outside: any = new Proxy(mid.proxy, {});
show("revoke_middle_before", outside.q);
mid.revoke();
attempt("revoke_middle_after", () => outside.q);
