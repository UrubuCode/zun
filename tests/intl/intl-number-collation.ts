// Cross-runtime: Intl.NumberFormat (en-US, no currency), Intl.Collator,
// Intl.getCanonicalLocales. Only cases verified byte-identical on Bun and Node.

const nf = new Intl.NumberFormat("en-US");
console.log("group=" + nf.format(1234567));
console.log("group-small=" + nf.format(999));
console.log("group-neg=" + nf.format(-1234));

const pct = new Intl.NumberFormat("en-US", { style: "percent" });
console.log("pct=" + pct.format(0.5));
console.log("pct-whole=" + pct.format(1));

const coll = new Intl.Collator("en");
console.log("cmp-lt=" + coll.compare("a", "b"));
console.log("cmp-gt=" + coll.compare("b", "a"));
console.log("cmp-eq=" + coll.compare("x", "x"));

const cb = new Intl.Collator("en", { sensitivity: "base" });
console.log("cmp-base=" + cb.compare("alpha", "ALPHA"));

console.log("canon=" + JSON.stringify(Intl.getCanonicalLocales("EN-us")));
console.log("canon-dup=" + JSON.stringify(Intl.getCanonicalLocales(["en-US", "en-us"])));

const resolved = new Intl.NumberFormat("en-US").resolvedOptions();
console.log("resolved-locale=" + resolved.locale);
console.log("resolved-style=" + resolved.style);
