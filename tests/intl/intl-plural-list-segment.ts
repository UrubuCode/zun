// Cross-runtime: Intl.PluralRules, Intl.ListFormat, Intl.RelativeTimeFormat,
// Intl.Segmenter (en / en-US). Only cases verified byte-identical on Bun and Node.

const pr = new Intl.PluralRules("en-US");
console.log("one=" + pr.select(1));
console.log("two=" + pr.select(2));
console.log("zero=" + pr.select(0));

const lf = new Intl.ListFormat("en-US", { style: "long", type: "conjunction" });
console.log("list=" + lf.format(["a", "b", "c"]));
console.log("list2=" + lf.format(["a", "b"]));

const rf = new Intl.RelativeTimeFormat("en-US", { numeric: "auto" });
console.log("rel=" + rf.format(-1, "day"));
console.log("rel-today=" + rf.format(0, "day"));

const seg = new Intl.Segmenter("en", { granularity: "word" });
const parts = Array.from(seg.segment("hi there")).map(
  (x: any) => x.segment + ":" + x.isWordLike
);
console.log("segments=" + parts.join("|"));
