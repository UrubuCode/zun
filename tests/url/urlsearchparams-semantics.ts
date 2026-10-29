// URLSearchParams: construction from every accepted shape, application/x-www-
// form-urlencoded parsing and serialisation, ordered mutation (set/append/
// delete/sort), iteration, and the live two-way link with URL.search.

const dump = function (label: string, p: URLSearchParams): void {
  const pairs: string[] = [];
  p.forEach(function (value, key) {
    pairs.push(JSON.stringify(key) + ":" + JSON.stringify(value));
  });
  console.log(label + " size=" + p.size + " str=" + JSON.stringify(p.toString()) + " pairs=[" + pairs.join(" ") + "]");
};

// --- Parsing input strings --------------------------------------------------
dump("plain", new URLSearchParams("a=1&b=2"));
dump("leading_mark", new URLSearchParams("?a=1&b=2"));
dump("two_marks", new URLSearchParams("??a=1"));
dump("empty_string", new URLSearchParams(""));
dump("only_mark", new URLSearchParams("?"));
dump("no_value", new URLSearchParams("a&b="));
dump("no_name", new URLSearchParams("=x&=y"));
dump("bare_equals", new URLSearchParams("="));
dump("empty_pairs", new URLSearchParams("a=1&&b=2&"));
dump("duplicate", new URLSearchParams("a=1&a=2&a=3"));
dump("extra_equals", new URLSearchParams("a=1=2"));
dump("multi_equals", new URLSearchParams("a=b=c"));
dump("percent_name", new URLSearchParams("a%20b=c%2Bd"));
dump("bad_percent", new URLSearchParams("a%zz=b%"));
dump("plus_pair", new URLSearchParams("a=b+c"));
dump("bare_keys", new URLSearchParams("a&b&c"));

// --- Decoding: + is a space, %XX is a byte, invalid escapes are kept --------
console.log("plus_decoded=" + JSON.stringify(new URLSearchParams("q=a+b").get("q")));
console.log("pct20_decoded=" + JSON.stringify(new URLSearchParams("q=a%20b").get("q")));
console.log("literal_plus=" + JSON.stringify(new URLSearchParams("q=a%2Bb").get("q")));
console.log("utf8_decoded=" + JSON.stringify(new URLSearchParams("q=%C3%A9%E2%82%AC").get("q")));
console.log("bad_escape=" + JSON.stringify(new URLSearchParams("q=%zz%").get("q")));
console.log("truncated_escape=" + JSON.stringify(new URLSearchParams("q=%C3").get("q")));
console.log("plus_in_name=" + JSON.stringify([...new URLSearchParams("a+b=1").keys()][0]));

// --- Construction from arrays, records, maps and other params ---------------
dump("array", new URLSearchParams([["k", "v"], ["k", "w"], ["z", "1"]]));
dump("map", new URLSearchParams(new Map([["b", "2"], ["a", "1"]]) as any));
dump("record", new URLSearchParams({ b: "2", a: "1", "1": "first" }));
dump("record_dupes", new URLSearchParams({ a: "1", A: "2" }));
dump("from_params", new URLSearchParams(new URLSearchParams("a=1&a=2")));
dump("no_arg", new URLSearchParams());
dump("undefined_arg", new URLSearchParams(undefined));
dump("coerced_record", new URLSearchParams({ n: 1 as any, b: true as any, u: null as any }));
dump("coerced_array", new URLSearchParams([["n", 1 as any], ["u", undefined as any]]));
console.log("from_pairs=" + new URLSearchParams([["a", "1"], ["b", "2"], ["a", "3"]]).toString());
console.log("from_record=" + new URLSearchParams({ b: "2", a: "1" }).toString());

// Malformed shapes throw TypeError instead of being ignored.
const bad: Array<[string, () => URLSearchParams]> = [
  ["short_pair", function () { return new URLSearchParams([["a"] as any]); }],
  ["long_pair", function () { return new URLSearchParams([["a", "b", "c"] as any]); }],
  ["flat_array", function () { return new URLSearchParams(["a", "b"] as any); }],
  ["number", function () { return new URLSearchParams(5 as any); }],
];
for (const b of bad) {
  try {
    console.log("ctor_" + b[0] + "=ok:" + JSON.stringify(b[1]().toString()));
  } catch (e: any) {
    console.log("ctor_" + b[0] + "=" + e.name + " instanceofTypeError=" + (e instanceof TypeError));
  }
}

// --- Serialisation: application/x-www-form-urlencoded (space is "+") --------
const round = function (label: string, k: string, v: string): void {
  const p = new URLSearchParams();
  p.append(k, v);
  const s = p.toString();
  const back = new URLSearchParams(s);
  console.log(label + " ser=" + JSON.stringify(s) + " backK=" + JSON.stringify(Array.from(back.keys())[0]) + " backV=" + JSON.stringify(back.get(k)));
};
round("space", "a b", "c d");
round("plus", "a+b", "c+d");
round("amp", "a&b", "c&d");
round("eq", "a=b", "c=d");
round("percent", "a%b", "c%d");
round("hash", "a#b", "c#d");
round("slash", "a/b", "c/d");
round("question", "a?b", "c?d");
round("tilde", "a~b", "c~d");
round("star", "a*b", "c*d");
round("bang", "a!b", "c!d");
round("paren", "a(b)", "c(d)");
round("quote", "a'b", "c'd");
round("dot", "a.b", "c.d");
round("dash", "a-b", "c-d");
round("under", "a_b", "c_d");
round("newline", "a\nb", "c\r\nd");
round("tab", "a\tb", "c\td");
round("accent", "é", "ü");
round("cjk", "中", "文");
round("astral", "\u{1F600}", "\u{1F601}");
round("empty", "", "");

const out = new URLSearchParams();
out.append("a b", "c d");
out.append("amp", "x&y=z");
out.append("plus", "1+1");
out.append("uni", "é€\u{1F600}");
out.append("safe", "*-._");
out.append("unsafe", "~!'()");
out.append("nl", "line1\nline2");
console.log("serialised=" + out.toString());

const lone = new URLSearchParams();
lone.append("a\uD800b", "c\uDC00d");
console.log("lone_surrogate_ser=" + JSON.stringify(lone.toString()));

// Which printable ASCII code points survive unescaped as a value.
const kept: string[] = [];
for (let c = 0x20; c < 0x7f; c++) {
  const ch = String.fromCharCode(c);
  const p = new URLSearchParams();
  p.append("k", ch);
  if (p.toString() === "k=" + ch) kept.push(ch);
}
console.log("unescaped_ascii=" + JSON.stringify(kept.join("")));

// Empty name and empty value survive a round trip.
const edge = new URLSearchParams();
edge.append("", "v");
edge.append("k", "");
edge.append("", "");
console.log("edge=" + JSON.stringify(edge.toString()));
console.log("edge_back=" + JSON.stringify(new URLSearchParams(edge.toString()).toString()));

// Every serialised pair reads back to the same value.
const back = new URLSearchParams(out.toString());
const same: string[] = [];
out.forEach(function (v, k) {
  same.push(String(back.getAll(k).indexOf(v) >= 0));
});
console.log("roundtrip=" + same.join(","));

// --- get / getAll / has with the optional value argument --------------------
const h = new URLSearchParams("a=1&a=2&b=");
console.log("get_first=" + h.get("a") + " getAll=" + h.getAll("a").join("|") + " size=" + h.size);
console.log("get_missing=" + String(h.get("z")) + " getAll_missing=" + JSON.stringify(h.getAll("z")));
console.log("has=" + h.has("a") + "," + h.has("z"));
console.log("has_value=" + h.has("a", "2") + "," + h.has("a", "3"));
console.log("has_empty_value=" + h.has("b", "") + "," + h.has("b", "x"));
console.log("get_empty_value=" + JSON.stringify(h.get("b")));

// --- set / append / delete ordering -----------------------------------------
const s = new URLSearchParams("a=1&b=2&a=3&c=4&a=5");
console.log("start=" + s.toString());
s.set("a", "X");
console.log("set_existing=" + s.toString() + " size=" + s.size);
s.set("z", "0");
console.log("set_new=" + s.toString());
s.append("b", "9");
console.log("append=" + s.toString());
s.set("a", "11");
console.log("set_collapses=" + s.toString());

const d = new URLSearchParams("a=1&b=2&a=2&a=1");
d.delete("a", "1");
console.log("delete_value=" + d.toString());
d.delete("a");
console.log("delete_name=" + d.toString());
d.delete("missing");
console.log("delete_missing=" + d.toString() + " size=" + d.size);
const dEmpty = new URLSearchParams("=x&a=1");
dEmpty.delete("");
console.log("delete_empty_name=" + dEmpty.toString());

// --- sort: by NAME code units only, stable for equal names ------------------
const so = new URLSearchParams("b=1&a=3&B=0&a=1&A=2&b=0&a=2");
console.log("sort_returns=" + String(so.sort()));
console.log("sort=" + so.toString());
const stable = new URLSearchParams("x=first&x=second&x=third&a=1");
stable.sort();
console.log("sort_stable=" + stable.toString());
const units = new URLSearchParams("é=1&z=2&Z=3&\u{1F600}=4&a=5");
units.sort();
console.log("sort_code_units=" + units.toString());

// --- Iteration: spread, for-of, entries/keys/values, forEach ----------------
const it = new URLSearchParams("a=1&b=2");
console.log("spread=" + JSON.stringify([...it]));
console.log("keys=" + [...it.keys()].join(",") + " values=" + [...it.values()].join(","));
console.log("entries_is_iterator=" + (typeof (it as any)[Symbol.iterator] === "function") + " same_as_entries=" + ((it as any)[Symbol.iterator] === it.entries));
console.log("tag=" + Object.prototype.toString.call(it) + " iterator_tag=" + Object.prototype.toString.call(it.entries()));
console.log("string_coerce=" + String(it));
console.log("object_from_entries=" + JSON.stringify(Object.fromEntries(new URLSearchParams("x=1&y=2"))));
const manual = new URLSearchParams('a=b+x&c=%20d&a=e').entries();
console.log("manual_next=" + JSON.stringify(manual.next().value) + " " + JSON.stringify(manual.next().value) + " done=" + manual.next().done + " " + manual.next().done);
const forOf: string[] = [];
for (const [k, v] of new URLSearchParams("p=1&q=2")) forOf.push(k + ":" + v);
console.log("for_of=" + forOf.join(","));
const live = new URLSearchParams("a=1&b=2");
const seen: string[] = [];
live.forEach(function (value, key, parent) {
  seen.push(key + "=" + value);
  if (key === "a") parent.append("c", "3");
});
console.log("forEach_sees_appends=" + seen.join(","));
console.log("forEach_third_arg_is_self=" + (function (): boolean {
  let ok = false;
  live.forEach(function (_v, _k, parent) {
    ok = parent === live;
  });
  return ok;
})());

// --- Live link: URL.search and URL.searchParams are two views of one list ---
const u = new URL("https://example.com/p?a=1&b=2#frag");
const params = u.searchParams;
console.log("same_object_each_read=" + (u.searchParams === params));
params.append("c", "3");
console.log("append_updates_search=" + u.search);
console.log("append_updates_href=" + u.href);
params.set("a", "9");
console.log("set_updates_search=" + u.search);
params.delete("b");
console.log("delete_updates_search=" + u.search);
u.search = "z=100&y=200";
console.log("search_updates_params=" + params.toString() + " still_same=" + (u.searchParams === params));
u.search = "";
console.log("cleared_params=" + JSON.stringify(params.toString()) + " href=" + u.href);
params.append("q", "a b");
console.log("reappend=" + u.search + " href=" + u.href);
params.delete("q");
console.log("emptied=" + JSON.stringify(u.search) + " href=" + u.href);
u.href = "https://example.com/x?k=v";
console.log("href_updates_params=" + params.toString() + " same=" + (u.searchParams === params));
const detached = new URLSearchParams("a=1");
detached.append("b", "2");
console.log("detached=" + detached.toString() + " unrelated_url=" + u.search);
u.search = "b=2&a=1&c=3";
params.sort();
console.log("sort_rewrites_href=" + u.href);
params.set("k", "x&y=z#w");
console.log("encoded_back=" + u.search);
console.log("decoded_again=" + JSON.stringify(u.searchParams.get("k")));
console.log("hash_untouched=" + JSON.stringify(u.hash));
