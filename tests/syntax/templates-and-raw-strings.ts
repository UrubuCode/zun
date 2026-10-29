// Template literals, tagged templates (cooked and raw), invalid escapes inside
// tagged templates, call-site caching of the strings object, and the Unicode
// escape forms of string literals.

// --- plain template literals: interpolation, nesting, escapes and multi-line text ---
function tagUpper(x: string): string {
  return x.toUpperCase();
}
const who = "rts";
const count = 2 + 3;
console.log(`basic=${who}:${count}`);
console.log(`nested=${`${who}-ok`}:${tagUpper("go")}`);
console.log(`escapes=line1\nline2\tend`);
console.log(`multi=a
b`);
console.log(`call=${tagUpper(who)}:${"x".repeat(2)}`);
const obj: any = { id: 7, ["user_" + 1]: "ana", nested: { value: 5 } };
console.log(`member=${obj.nested.value}:${obj["user_" + 1]}:${obj.id}`);
const dyn = "power";
const dynamicObj: any = { [dyn]: 9 };
console.log(`dynamic_key=${dynamicObj[dyn]}|${[1, 2, 3]}|${{ toString: () => "custom" }}`);
console.log(`empty_interp=[${""}]|${null}|${undefined}`);

// --- tagged templates receive cooked strings and raw strings in parallel ---
function tag(strings: TemplateStringsArray, ...values: unknown[]): string {
  return strings.raw.join("|") + "::" + values.join(",");
}
const value = 42;
console.log("raw_string=" + String.raw`line1\n${value}\tend`);
console.log("tag_raw=" + tag`a\n${value}b\t${"x"}`);
console.log("string_raw_plain=" + String.raw`a\nb`.length);
console.log("string_raw_tab=" + String.raw`tab\there`);
function highlight(strings: TemplateStringsArray, ...values: any[]): string {
  return strings.reduce((acc, str, i) => acc + str + (values[i] !== undefined ? "[" + values[i] + "]" : ""), "");
}
const name = "world";
const n = 42;
console.log("highlight=" + highlight`hello ${name} the number is ${n}`);
console.log("highlight_none=" + highlight`no interpolation`);
console.log("highlight_adjacent=" + highlight`${1}${2}${3}`);
function parts(strings: TemplateStringsArray, ...vals: any[]): string[] {
  return [...strings];
}
console.log("parts=" + parts`a${1}b${2}c`.join("|"));
function rawVsCooked(strings: TemplateStringsArray): string {
  return strings[0] + " / " + strings.raw[0];
}
console.log("raw_vs_cooked=" + rawVsCooked`line1\nline2`);
function parallel(parts2: TemplateStringsArray, ...values: any[]): string {
  return parts2.map((p, i) => p + "/" + parts2.raw[i] + "/" + (values[i] ?? "")).join("|");
}
console.log("parallel=" + parallel`a\nb${7}c\td`);

// --- invalid escapes: legal only in tagged templates; the cooked entry becomes undefined ---
function show(strings: TemplateStringsArray, ...values: any[]): string {
  const cooked = strings.map((s) => (s === undefined ? "<undef>" : JSON.stringify(s))).join(" ");
  const raw = strings.raw.map((s) => JSON.stringify(s)).join(" ");
  return "cooked[" + cooked + "] raw[" + raw + "] vals[" + values.join(",") + "]";
}
console.log("legal_escapes=" + show`a\nb`);
console.log("legal_tab_cr=" + show`a\tb` + "|" + show`a\rb`);
console.log("legal_codepoint=" + show`a\u{42}b`);
console.log("legal_hex=" + show`a\x41b`);
console.log("legal_backslash=" + show`a\\b`);
console.log("bad_u_word=" + show`\unicode`);
console.log("bad_u_short=" + show`\u12`);
console.log("bad_u_brace_empty=" + show`\u{}`);
console.log("bad_u_brace_bad=" + show`\u{zz}`);
console.log("bad_x=" + show`\xZZ`);
console.log("bad_x_short=" + show`\x1`);
console.log("bad_octal=" + show`\01`);
console.log("bad_octal_8=" + show`\08`);
console.log("one_bad_segment=" + show`ok\t${1}\unicode${2}fine\n`);
console.log("first_bad=" + show`\xZZ${1}good`);
console.log("last_bad=" + show`good${1}\u{`);
console.log("mixed_segment=" + show`\n\unicode\t`);
console.log("mixed_valid_first=" + show`A\u{zz}`);
function rawOnly(strings: TemplateStringsArray): string {
  return strings.raw.join("|");
}
console.log("raw_word=" + rawOnly`\unicode` + "|len=" + rawOnly`\unicode`.length);
console.log("raw_octal=" + rawOnly`\01`);
console.log("string_raw_bad=" + String.raw`\unicode\xZZ` + "|" + String.raw`\01\02`);
function shapeOf(strings: TemplateStringsArray, ...values: any[]): string {
  return strings.length + "/" + strings.raw.length + "/" + values.length +
    " undef=" + strings.map((s) => (s === undefined ? "1" : "0")).join("");
}
console.log("shape_all_good=" + shapeOf`a${1}b${2}c`);
console.log("shape_middle_bad=" + shapeOf`a${1}\unicode${2}c`);
console.log("shape_all_bad=" + shapeOf`\u{${1}\xZ${2}\01`);
function joinRaw(strings: TemplateStringsArray, ...values: any[]): string {
  let out = strings.raw[0];
  for (let i = 0; i < values.length; i++) out += String(values[i]) + strings.raw[i + 1];
  return out;
}
console.log("join_raw=" + joinRaw`pre\unicode${42}post\01`);
const untaggedLegal = JSON.stringify(`a\nb`) + "|" + JSON.stringify(`a\x41b`);
console.log("untagged_legal=" + untaggedLegal);

// --- the strings object is cached per call site, frozen, and refuses writes ---
const seen: any[] = [];
function grab(strings: TemplateStringsArray): any { return strings; }
for (let i = 0; i < 2; i++) seen.push(grab`\unicode`);
console.log("cached=" + (seen[0] === seen[1]) + "|frozen=" + Object.isFrozen(seen[0]));
console.log("cooked_undefined=" + (seen[0][0] === undefined) + "|raw=" + JSON.stringify(seen[0].raw[0]));
const fromLoop: any[] = [];
for (let i = 0; i < 3; i++) fromLoop.push(grab`a${i}b`);
console.log("loop_same_site=" + (fromLoop[0] === fromLoop[1]) + "|" + (fromLoop[1] === fromLoop[2]));
const siteA = grab`a${1}b`;
const siteB = grab`a${1}b`;
console.log("distinct_sites=" + (siteA === siteB));
function callSite(k: number): any { return grab`x${k}y`; }
const c1 = callSite(1);
const c2 = callSite(2);
console.log("function_site=" + (c1 === c2) + "|" + (c1 === siteA));
function recurse(k: number, acc: any[]): any[] {
  acc.push(grab`r${k}`);
  return k > 0 ? recurse(k - 1, acc) : acc;
}
const rec = recurse(3, []);
console.log("recursion_same=" + (rec[0] === rec[1] && rec[1] === rec[2] && rec[2] === rec[3]));
function tagOne(strings: TemplateStringsArray): any { return strings; }
function tagTwo(strings: TemplateStringsArray): any { return strings; }
const which: any[] = [];
for (let i = 0; i < 2; i++) {
  const fn = i === 0 ? tagOne : tagTwo;
  which.push(fn`shared${i}`);
}
console.log("cache_by_site_not_tag=" + (which[0] === which[1]));
const s = grab`p${1}q${2}r`;
console.log("shape=" + Array.isArray(s) + "|" + s.length + "|" + s.join("/") + "|" + Array.isArray(s.raw) + "|" + s.raw.length + "|" + (s.raw === s));
console.log("frozen_flags=" + Object.isFrozen(s) + "|" + Object.isFrozen(s.raw) + "|" + Object.isSealed(s) + "|" + Object.isExtensible(s));
console.log("set_refused=" + Reflect.set(s, 0, "changed") + "|" + Reflect.set(s, "raw", []) + "|" + Reflect.set(s, "extra", 1));
console.log("delete_define_refused=" + Reflect.deleteProperty(s, 0) + "|" + Reflect.defineProperty(s, 0, { value: "changed" }));
console.log("unchanged=" + s[0] + "|" + s.raw[0] + "|extra=" + ("extra" in s));
const zeroDesc = Object.getOwnPropertyDescriptor(s, 0) as any;
console.log("index_desc=" + zeroDesc.writable + "/" + zeroDesc.configurable + "|not_extensible=" + !Reflect.isExtensible(s));
const rawDesc = Object.getOwnPropertyDescriptor(s, "raw") as any;
console.log("raw_desc=" + Object.prototype.hasOwnProperty.call(s, "raw") + "|" + rawDesc.enumerable + "|" + rawDesc.writable + "|" + rawDesc.configurable);
console.log("own_keys=" + Object.getOwnPropertyNames(s).join(","));
const collected: string[] = [];
function collect(strings: TemplateStringsArray, ...values: any[]): string {
  collected.push(values.join("+"));
  return strings.raw.join("_");
}
for (let i = 0; i < 3; i++) collect`v${i}w${i * 2}`;
console.log("values_not_cached=" + collected.join("|"));

// --- Unicode escapes in string literals: \uXXXX is one code unit, \u{...} is one code point ---
function codes(str: string): string {
  return str.split("").map((c) => c.charCodeAt(0).toString(16)).join(",");
}
console.log("hex4=" + "\u0041" + "|" + "\u0061" + "|" + "\u00E9" + "|" + "\u00e9");
console.log("hex4_case_eq=" + ("\u00E9" === "\u00e9") + "|max=" + "\uFFFF".charCodeAt(0).toString(16) + "|nul_len=" + "\u0000".length);
console.log("brace=" + "\u{41}" + "|" + "\u{041}" + "|" + "\u{00041}" + "|" + "\u{000041}");
console.log("brace_astral=" + "\u{1F600}".length + "|" + "\u{1F600}".codePointAt(0)!.toString(16));
console.log("brace_limits=" + "\u{10FFFF}".length + "|" + "\u{FFFF}".length);
const pairLit = "\uD83D\uDE00";
console.log("pair_literal=" + pairLit.length + "|" + codes(pairLit) + "|" + (pairLit === "\u{1F600}"));
const mix2 = "\u{1F600}\uDE00";
console.log("mix_brace_lo=" + mix2.length + "|" + codes(mix2));
console.log("mix_text=" + "\u0041B\u0043" + "|" + "\u{41}B\u{43}" + "|" + "\u0041\u{42}C");
console.log("adjacent=" + "a\u0042c\u{44}e" + "|" + "\u0041" + "1" + "|" + "\u{41}" + "1" + "|" + "\u{41}{}");
console.log("tpl_escapes=" + `\u0041` + "|" + `\u{42}` + "|" + `\uD83D\uDE00`.length + "|" + `${"\u0041"}\u{42}`);
const escKeys: any = { "\u0041": 1, "\u{42}": 2 };
console.log("escaped_keys=" + escKeys.A + "|" + escKeys.B);
console.log("lone_surrogates=" + "\uD83D".length + "|" + "\u{D83D}".length + "|" + ("\uD83D" === "\u{D83D}"));
const run = "A\u{1F600}B\uD83D\uDE01C";
console.log("run=" + run.length + "|" + [...run].length);
