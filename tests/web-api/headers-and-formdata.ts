// Web Headers and FormData: case-insensitive names, combined values, set-cookie
// handling, value trimming, HTTP-grammar validation, and ordered entry iteration.

const showHeaders = function (label: string, h: Headers): void {
  const parts: string[] = [];
  h.forEach(function (value, key) {
    parts.push(key + ": " + value);
  });
  console.log(label + " [" + parts.join(" | ") + "]");
};

const showForm = function (label: string, fd: FormData): void {
  const parts: string[] = [];
  fd.forEach(function (value, key) {
    parts.push(key + "=" + String(value));
  });
  console.log(label + " [" + parts.join(" | ") + "]");
};

// ---- Headers: names fold to lower case whatever the input spelling. ----
const h = new Headers();
h.append("Content-Type", "text/plain");
h.append("X-CUSTOM", "one");
h.append("x-custom", "two");
h.append("Accept", "a/b");
console.log("get_any_case=" + h.get("CONTENT-TYPE") + " | " + h.get("content-type") + " | " + h.get("Content-Type"));
console.log("combined=" + h.get("x-custom"));
console.log("has=" + h.has("X-Custom") + "," + h.has("x-custom") + "," + h.has("missing"));
showHeaders("sorted", h);
console.log("keys=" + [...h.keys()].join(","));
console.log("values=" + [...h.values()].join(" / "));
console.log("entries=" + [...h.entries()].map(function (p) { return p[0] + "=" + p[1]; }).join(" / "));
console.log("spread_matches_entries=" + (JSON.stringify([...h]) === JSON.stringify([...h.entries()])));
console.log("iterator_is_entries=" + ((Headers.prototype as any)[Symbol.iterator] === Headers.prototype.entries));

// A repeated append keeps adding to the same slot.
h.append("x-custom", "three");
console.log("three_values=" + h.get("x-custom"));
console.log("one_key_slot=" + [...h.keys()].filter(function (k) { return k === "x-custom"; }).length);

// set() replaces every value at once and keeps the sorted position.
h.set("X-Custom", "only");
console.log("after_set=" + h.get("x-custom"));
showHeaders("after_set_order", h);
h.set("A-First", "1");
showHeaders("after_new_set", h);

// delete() is case-insensitive and silent when the name is absent.
h.delete("A-FIRST");
h.delete("not-there");
console.log("after_delete=" + [...h.keys()].join(","));

// Values lose surrounding HTTP whitespace, never inner whitespace.
const t = new Headers();
t.append("a", "  spaced  ");
t.append("b", "\ttabbed\t");
t.append("c", "in  ner");
console.log("trimmed=" + JSON.stringify(t.get("a")) + "," + JSON.stringify(t.get("b")) + "," + JSON.stringify(t.get("c")));
t.append("a", "  more  ");
console.log("trimmed_join=" + JSON.stringify(t.get("a")));
console.log("empty_value=" + JSON.stringify(new Headers({ e: "" }).get("e")));
console.log("inner_tab=" + JSON.stringify(new Headers({ x: "a\tb" }).get("x")));

// Non-string arguments are coerced with ToString.
const c = new Headers();
c.append("x-num", 42 as any);
c.append("x-bool", true as any);
console.log("coerced=" + c.get("x-num") + "," + c.get("x-bool"));

// Set-Cookie is kept as separate entries; getSetCookie() returns them unjoined.
const sc = new Headers();
sc.append("set-cookie", "a=1");
sc.append("set-cookie", "b=2");
console.log("getSetCookie_type=" + typeof (sc as any).getSetCookie);
console.log("getSetCookie=" + (sc as any).getSetCookie().join("|"));
console.log("set_cookie_get=" + sc.get("set-cookie"));

// Illegal names and values throw a TypeError; the list is left unchanged.
const badNames: string[] = ["a b", "a:b", "", "a\nb", "a\tb", "a(b", "a,b", "a@b", "a{b", "a\"b", "a/b", "a[b"];
for (const name of badNames) {
  try {
    new Headers().set(name, "1");
    console.log("name[" + JSON.stringify(name) + "]=accepted");
  } catch (e: any) {
    console.log("name[" + JSON.stringify(name) + "]=" + e.constructor.name);
  }
}
const goodNames: string[] = ["a-b", "a_b", "a.b", "a~b", "a!b", "a#b", "a$b", "a%b", "a&b", "a'b", "a*b", "a+b", "a^b", "a`b", "a|b", "0"];
console.log("accepted_names=" + goodNames.filter(function (n) {
  try {
    new Headers().set(n, "1");
    return true;
  } catch (e: any) {
    return false;
  }
}).join(","));

const badValues: string[] = ["v\n1", "v\r1", "v\u00001", "\nv", "v\u0000", "\u0000v"];
for (const value of badValues) {
  try {
    new Headers().set("x", value);
    console.log("value[" + JSON.stringify(value) + "]=accepted");
  } catch (e: any) {
    console.log("value[" + JSON.stringify(value) + "]=" + e.constructor.name);
  }
}

// A missing name answers null, not undefined.
const empty = new Headers();
console.log("missing=" + String(empty.get("nope")) + " typeof=" + typeof empty.get("nope"));
console.log("empty_iteration=" + JSON.stringify([...empty]) + " keys=" + [...empty.keys()].length);

// Construction from a record, an array of pairs, and another Headers.
showHeaders("from_record", new Headers({ "Z-Last": "1", "A-First": "2" }));
showHeaders("from_pairs", new Headers([["z-last", "1"], ["a-first", "2"], ["z-last", "3"]]));
showHeaders("from_headers", new Headers(new Headers({ b: "2", a: "1" })));
try {
  new Headers([["only-one"] as any]);
  console.log("bad_pair=accepted");
} catch (e: any) {
  console.log("bad_pair=" + e.constructor.name);
}
console.log("tag=" + Object.prototype.toString.call(h));

// ---- FormData: ordered entries, repeated keys, and replacement. ----
const fd = new FormData();
fd.append("a", "1");
fd.append("a", "2");
fd.append("b", "x");
console.log("fd_get=" + fd.get("a"));
console.log("fd_getAll=" + fd.getAll("a").join(","));
console.log("fd_entries=" + Array.from(fd.entries()).map(function (p: any) { return p[0] + "=" + p[1]; }).join("|"));
console.log("fd_keys=" + Array.from(fd.keys()).join(","));
console.log("fd_values=" + Array.from(fd.values()).join(","));
console.log("fd_has=" + fd.has("a") + "," + fd.has("zzz"));
console.log("fd_get_missing=" + String(fd.get("zzz")));
console.log("fd_getAll_missing=" + fd.getAll("zzz").length);
showForm("fd_forEach", fd);

// set() collapses every value of a key into one, at the first position.
fd.append("c", "3");
fd.set("a", "9");
console.log("fd_after_set=" + fd.getAll("a").join(",") + " order=" + Array.from(fd.keys()).join(","));
fd.append("b", "y");
fd.delete("b");
console.log("fd_after_delete=" + Array.from(fd.keys()).join(",") + " has_b=" + fd.has("b"));
console.log("fd_tag=" + Object.prototype.toString.call(fd));
console.log("fd_iterator_is_entries=" + ((FormData.prototype as any)[Symbol.iterator] === FormData.prototype.entries));
