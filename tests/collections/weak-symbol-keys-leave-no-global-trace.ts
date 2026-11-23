// Cross-runtime: using unregistered symbols as WeakMap/WeakSet keys must leave no
// trace on any object user code can reach. The engine keeps a per-symbol key cell;
// it must not be observable through the realm global's own keys.

function run(): void {
  const g: any = globalThis;
  const snapshot = (): string[] => {
    const own = Object.getOwnPropertyNames(g).map((k) => "n:" + k);
    const keys = Object.keys(g).map((k) => "k:" + k);
    const looped: string[] = [];
    for (const k in g) looped.push("i:" + k);
    return own.concat(keys, looped).sort();
  };
  const before = snapshot();
  const a = Symbol("a");
  const b = Symbol("a");
  const wm = new WeakMap<any, string>([[a, "first"], [b, "second"]] as any);
  wm.set(Symbol("tmp"), "dropped");
  const ws = new WeakSet<any>([a] as any);
  console.log("values=" + wm.get(a) + ":" + wm.get(b) + ":" + ws.has(a) + ":" + ws.has(b));
  const after = snapshot();
  const added = after.filter((k) => before.indexOf(k) < 0);
  console.log("added=" + added.length + ":" + added.join(","));
  console.log("internal=" + after.filter((k) => k.indexOf("symcell") >= 0 || k.indexOf("\u0001") >= 0).length);
}
run();
