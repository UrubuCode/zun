// Percent-encoding and base64 text transforms: the four URI functions and their
// unreserved/reserved split, URIError on malformed input, the legacy escape and
// unescape pair (UTF-16 code units), and btoa/atob over the Latin-1 binary string
// with forgiving-base64 rules. Every line is a deterministic label=value pair.

const codes = function (s: string): string {
  const out: string[] = [];
  for (let i = 0; i < s.length; i++) out.push(s.charCodeAt(i).toString(16));
  return out.join(",");
};
const t = function (f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.constructor.name + "/" + e.name;
  }
};

// Printable ASCII (0x21..0x7e) that a function leaves untouched.
const untouched = function (fn: (s: string) => string): string {
  let out = "";
  for (let i = 0x21; i < 0x7f; i++) {
    const ch = String.fromCharCode(i);
    if (fn(ch) === ch) out += ch;
  }
  return out;
};

// ---------------------------------------------------------------------------
// Unreserved sets. encodeURIComponent keeps A-Za-z0-9 and -_.!~*'() only.
// encodeURI additionally keeps the reserved set ;/?:@&=+$,#.
// ---------------------------------------------------------------------------
console.log("euc_keeps=" + untouched(encodeURIComponent));
console.log("eu_keeps=" + untouched(encodeURI));
console.log("esc_keeps=" + untouched(escape));
console.log("euc_minus_eu=" + (function (): string {
  let out = "";
  for (let i = 0x21; i < 0x7f; i++) {
    const ch = String.fromCharCode(i);
    if (encodeURI(ch) === ch && encodeURIComponent(ch) !== ch) out += ch;
  }
  return out;
})());
console.log("space=" + encodeURIComponent(" ") + "," + encodeURI(" ") + "," + escape(" "));
console.log("newline=" + encodeURIComponent("\n") + "," + escape("\n"));
console.log("plus=" + encodeURIComponent("+") + " slash=" + encodeURIComponent("/") + " question=" + encodeURIComponent("?") + " amp=" + encodeURIComponent("&"));
console.log("encoded=" + encodeURI("https://example.com/path with spaces"));
console.log("component=" + encodeURIComponent("hello world & foo=bar"));
console.log("mixed=" + encodeURI("https://\u00e9.example/p a?q=\u00e9#f"));

// Non-ASCII becomes percent-encoded UTF-8 under both encoders.
console.log("latin=" + encodeURIComponent("\u00e9") + "," + encodeURI("\u00e9"));
console.log("euro=" + encodeURIComponent("\u20ac"));
console.log("astral=" + encodeURIComponent("\u{1f600}") + " valid_pair=" + encodeURIComponent("\u{1f600}"));

// Coercion through ToString, and the reserved characters of decodeURI.
console.log("coerce=" + encodeURIComponent(1.5 as any) + "," + encodeURIComponent(null as any) + "," + encodeURIComponent(undefined as any));
console.log("coerce_object=" + encodeURIComponent({} as any) + " coerce_array=" + encodeURIComponent(["a b", "c"] as any));

// ---------------------------------------------------------------------------
// Decoding: decodeURIComponent undoes everything; decodeURI keeps the reserved
// set encoded, so it is NOT the inverse of encodeURIComponent.
// ---------------------------------------------------------------------------
const reserved = "%23%24%26%2B%2C%2F%3A%3B%3D%3F%40";
console.log("duc_reserved=" + decodeURIComponent(reserved));
console.log("du_reserved=" + decodeURI(reserved));
console.log("du_unreserved=" + decodeURI("%41%20%C3%A9"));
console.log("duc_utf8=" + decodeURIComponent("%E2%82%AC%F0%9F%98%80"));
console.log("not_inverse=" + (decodeURI(encodeURIComponent("a/b")) === "a/b"));
console.log("is_inverse=" + (decodeURIComponent(encodeURIComponent("a/b?c#d")) === "a/b?c#d"));
console.log("decode_plus_kept=" + decodeURIComponent("a+b%20c"));

// Malformed sequences raise a URIError from both decoders.
const malformed: string[] = ["%", "%z", "%zz", "%4", "%C3", "%C3%28", "%C0%80", "%E0%A4", "%F5%80%80%80", "%ED%A0%80", "%FF"];
for (const s of malformed) {
  console.log("bad[" + s + "] duc=" + t(function () { return decodeURIComponent(s); }) + " du=" + t(function () { return decodeURI(s); }));
}

// A lone surrogate cannot be encoded as UTF-8: encodeURI* throws, escape does not.
const lone: string[] = ["\uD800", "\uDC00", "a\uD800b", "\uD800\uD800"];
for (const s of lone) {
  console.log("lone[" + (s.codePointAt(0) as number).toString(16) + ":" + s.length + "] euc=" + t(function () { return encodeURIComponent(s); }) + " escape=" + escape(s));
}

// Names and arities of the oldest globals.
console.log("arity=" + encodeURI.length + "," + encodeURIComponent.length + "," + decodeURI.length + "," + decodeURIComponent.length + "," + escape.length + "," + unescape.length);
console.log("names=" + [encodeURI.name, encodeURIComponent.name, decodeURI.name, decodeURIComponent.name, escape.name, unescape.name].join(","));

// ---------------------------------------------------------------------------
// escape/unescape: the Annex B pair. They work on UTF-16 CODE UNITS: %XX below
// U+0100, %uXXXX above it. unescape leaves a malformed escape where it found it.
// ---------------------------------------------------------------------------
console.log("types=" + typeof escape + "," + typeof unescape);
console.log("escape_ascii=" + escape("abc ABC 019") + " punct=" + escape("@*_+-./") + " other=" + escape("!~'()"));
console.log("escape_reserved=" + escape(";/?:@&=+$,#") + " quotes=" + escape("\"'`"));
console.log("escape_controls=" + escape("\u0000\u000a\u001f") + " del=" + escape("\u007f") + " high_ascii=" + escape("\u0080\u00ff"));
console.log("escape_boundary=" + escape("\u00ff\u0100"));
console.log("escape_bmp=" + escape("\u20ac\ud7ff\ue000\uffff"));
console.log("escape_astral=" + escape("\u{1f600}"));
console.log("escape_lone_surrogate=" + escape("\ud800") + "," + escape("\udfff"));
console.log("escape_empty=" + JSON.stringify(escape("")));
console.log("escape_coerces=" + escape(123 as any) + "," + escape(null as any) + "," + escape(true as any));
console.log("escape_uses_upper_hex=" + escape("\u00ab") + " " + (escape("\u00ab") === escape("\u00ab").toUpperCase()));
console.log("escape_latin=" + escape("\u00e9") + " euro=" + escape("\u20ac") + " astral=" + escape("\u{1f600}"));
console.log("unescape_hex=" + codes(unescape("%41%42")) + " u=" + codes(unescape("%u0041%u20AC")));
console.log("unescape_lowercase_u=" + codes(unescape("%u00e9")) + " upper_hex=" + codes(unescape("%E9")));
console.log("unescape_mixed=" + codes(unescape("A%42%u0043D")));
console.log("unescape_incomplete=" + codes(unescape("%4")) + "," + codes(unescape("%")) + "," + codes(unescape("%u")) + "," + codes(unescape("%u00")));
console.log("unescape_bad_hex=" + codes(unescape("%zz")) + "," + codes(unescape("%uzzzz")) + "," + codes(unescape("%4z")));
console.log("unescape_percent_only=" + codes(unescape("100%")) + " trailing=" + codes(unescape("a%")));
console.log("unescape_double=" + codes(unescape("%2541")));
console.log("unescape_plain=" + JSON.stringify(unescape("hello+world")) + " empty=" + JSON.stringify(unescape("")));
console.log("unescape_coerces=" + JSON.stringify(unescape(123 as any)) + "," + JSON.stringify(unescape(null as any)));
console.log("unescape_never_throws=" + t(function () { return JSON.stringify(unescape("%u{1f600}%%%")); }));
console.log("roundtrip_latin1=" + (unescape(escape("\u00e9 \u00ff")) === "\u00e9 \u00ff"));
console.log("roundtrip_astral=" + (unescape(escape("\u{1f600}")) === "\u{1f600}"));
console.log("roundtrip_mixed=" + (unescape(escape("a\u00e9\u20ac\u{1f600}")) === "a\u00e9\u20ac\u{1f600}"));
console.log("roundtrip_all_bmp=" + (function (): string {
  let src = "";
  for (let i = 0; i < 0x300; i++) src += String.fromCharCode(i);
  return String(unescape(escape(src)) === src);
})());
console.log("vs_encodeURIComponent=" + escape("\u00e9") + " vs " + encodeURIComponent("\u00e9"));
console.log("vs_astral=" + escape("\u{1f600}") + " vs " + encodeURIComponent("\u{1f600}"));
console.log("vs_plus=" + escape("+") + " vs " + encodeURIComponent("+") + " vs_slash=" + escape("/") + " vs " + encodeURIComponent("/"));
console.log("decodeURIComponent_of_escape=" + t(function () { return decodeURIComponent(escape("\u00e9")); }));
console.log("unescape_of_encodeURIComponent=" + codes(unescape(encodeURIComponent("\u00e9"))));
console.log("escape_descriptor=" + (function (): string {
  const d: any = Object.getOwnPropertyDescriptor(globalThis, "escape");
  return d ? "w:" + d.writable + " e:" + d.enumerable + " c:" + d.configurable : "<absent>";
})());

// ---------------------------------------------------------------------------
// btoa and atob work on a BINARY STRING (one code unit per byte, U+0000..U+00FF).
// Errors are DOMException InvalidCharacterError.
// ---------------------------------------------------------------------------
console.log("btoa_empty=" + JSON.stringify(btoa("")));
console.log("btoa_lengths=" + btoa("a") + "," + btoa("ab") + "," + btoa("abc") + "," + btoa("abcd"));
console.log("btoa_nul=" + btoa("\u0000") + " nul_pair=" + btoa("\u0000\u0000"));
console.log("btoa_high=" + btoa("\u00ff\u00fe") + " boundary=" + btoa("\u00ff"));
console.log("btoa_all_bits=" + btoa("\u0000\u0001\u00fe\u00ff"));
console.log("btoa_coerces=" + btoa(123 as any) + "," + btoa(null as any) + "," + btoa(true as any) + "," + btoa({ toString: function () { return "A"; } } as any));
console.log("btoa_no_arg=" + t(function () { return (btoa as any)(); }));

// One code unit above the Latin-1 window refuses the whole call.
const overRange: string[] = ["\u0100", "\u20ac", "a\u0100", "\u{1F600}", "\uD800"];
for (const s of overRange) {
  console.log("btoa_over[" + s.length + ":" + (s.codePointAt(0) as number).toString(16) + "]=" + t(function () { return btoa(s); }));
}
console.log("btoa_lone_surrogate=" + t(function () { return btoa("\ud800"); }));
console.log("btoa_error_is_domexception=" + (function (): string {
  try {
    btoa("\u0100");
    return "no-throw";
  } catch (e: any) {
    return (e instanceof DOMException) + "/" + (e instanceof Error) + "/" + e.name + "/" + e.code;
  }
})());

// atob: forgiving base64. ASCII whitespace is stripped anywhere, padding is checked
// after that, and a remainder of one character is always an error.
const dec = function (s: any): string {
  return t(function () { return codes(atob(s)); });
};
console.log("atob_padded=" + dec("YWJj") + " " + dec("YWI=") + " " + dec("YQ=="));
console.log("atob_unpadded=" + dec("YWJj") + " " + dec("YWI") + " " + dec("YQ"));
console.log("pad_variants=" + dec("QQ==") + " " + dec("QUI=") + " " + dec("QUJD"));
console.log("wrong_pad_count=" + dec("QQ=") + " " + dec("QUI==") + " " + dec("QUJD="));
console.log("overpadded=" + dec("QQ===") + " " + dec("QQ===="));
console.log("remainder_one=" + dec("Q") + " " + dec("QUJDQ") + " " + dec("a") + " " + dec("abcde"));
console.log("all_padding=" + dec("====") + " " + dec("="));
console.log("empty=" + JSON.stringify(atob("")) + " length=" + atob("").length);
console.log("pad_in_middle=" + dec("QQ==QQ==") + " " + dec("=QQQ") + " " + dec("QQ=Q") + " " + dec("YWJ=j") + " " + dec("YQ=="));
console.log("ws_around=" + dec(" QQ== "));
console.log("ws_inside=" + dec("Q Q = ="));
console.log("ws_kinds=" + dec("\tQ\nQ\r=\f="));
console.log("ws_only=" + dec("   ") + " length=" + atob("  \t\n").length + " ws_text=" + JSON.stringify(atob("   ")));
console.log("ws_makes_legal=" + dec("Q Q") + " ws_cannot_fix_remainder=" + dec("Q "));
console.log("nbsp_not_ws=" + dec("\u00a0QQ==") + " vertical_tab=" + dec("\u000bQQ=="));
console.log("atob_spaces=" + atob("  Y W\tJ\nj\r  ") + " ws_in_padding=" + atob("YQ = ="));
console.log("plus_slash=" + dec("+/8=") + " url_safe=" + dec("-_8=") + " full_range=" + dec("/w==") + " " + dec("AA==") + " " + dec("AAAA"));
console.log("bad_char=" + dec("Q*==") + " " + dec("Q.A=") + " " + dec("QQ@=") + " " + dec("@@@@") + " " + dec("-_"));
console.log("bad_extra=" + dec("YQ===") + " " + dec("YWJj=") + " " + dec("YQ=") + " " + dec("===="));
console.log("non_ascii=" + dec("QQ\u00e9=") + " " + dec("\u{1f600}QQ="));
console.log("high_bytes=" + dec("gIH+/w=="));
console.log("nul_byte=" + dec("AEEA") + " length=" + atob("AEEA").length);
console.log("length_by_group=" + atob("QUJD").length + "," + atob("QUI=").length + "," + atob("QQ==").length);
console.log("atob_high_byte=" + atob("/w==").charCodeAt(0) + " len=" + atob("/w==").length);
console.log("atob_coerces=" + dec({ toString: function () { return "QQ=="; } }) + " " + dec(null) + " " + dec(1234) + " " + dec(["QQ=="]));
console.log("atob_no_arg=" + t(function () { return (atob as any)(); }));
console.log("atob_latin1=" + t(function () { return atob("\u00e9"); }));

// Round trip: every byte 0..255 survives both directions.
let binary = "";
for (let i = 0; i < 256; i++) binary += String.fromCharCode(i);
const encoded = btoa(binary);
console.log("roundtrip_len=" + encoded.length + " equal=" + (atob(encoded) === binary));
console.log("roundtrip_head=" + encoded.slice(0, 8) + " tail=" + encoded.slice(-8));

// Text goes through UTF-8 first; base64 then carries the bytes.
const utf8 = new TextEncoder().encode("h\u00e9llo \u20ac");
let asBinary = "";
for (let i = 0; i < utf8.length; i++) asBinary += String.fromCharCode(utf8[i]);
const b64 = btoa(asBinary);
console.log("utf8_b64=" + b64);
const back = atob(b64);
const rebuilt = new Uint8Array(back.length);
for (let i = 0; i < back.length; i++) rebuilt[i] = back.charCodeAt(i);
console.log("utf8_roundtrip=" + new TextDecoder().decode(rebuilt));

// btoa/atob are ordinary writable, configurable globals.
const bd = Object.getOwnPropertyDescriptor(globalThis, "btoa") as any;
console.log("btoa_descriptor=w:" + bd.writable + " e:" + bd.enumerable + " c:" + bd.configurable);
console.log("b64_lengths=" + btoa.length + "," + atob.length + " names=" + btoa.name + "," + atob.name);
