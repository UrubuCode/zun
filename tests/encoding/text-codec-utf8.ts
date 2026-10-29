// TextEncoder and the UTF-8 side of TextDecoder: width boundaries, lone surrogates,
// encodeInto partial writes, malformed input (lossy vs fatal), BOM handling and
// streaming. Every line is a deterministic label=value pair.

const enc = new TextEncoder();
const dec = new TextDecoder();

const hex = function (u: Uint8Array): string {
  const out: string[] = [];
  for (let i = 0; i < u.length; i++) out.push(u[i].toString(16).padStart(2, "0"));
  return out.join(" ");
};
const units = function (s: string): string {
  const out: string[] = [];
  for (let i = 0; i < s.length; i++) out.push(s.charCodeAt(i).toString(16));
  return out.join(",");
};
const cps = function (s: string): string {
  const out: string[] = [];
  for (const ch of s) out.push((ch.codePointAt(0) as number).toString(16));
  return out.join(",");
};
const bytes = function (...v: number[]): Uint8Array {
  return new Uint8Array(v);
};
const t = function (f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.name;
  }
};

// ---------------------------------------------------------------------------
// Encoder: the 1/2/3/4-byte frontiers and the surrogate hole.
// ---------------------------------------------------------------------------
console.log("encoding=" + enc.encoding + " tag=" + Object.prototype.toString.call(enc));

const boundaries = [0x00, 0x7f, 0x80, 0x7ff, 0x800, 0xd7ff, 0xe000, 0xffff, 0x10000, 0x10ffff];
for (const cp of boundaries) {
  const s = String.fromCodePoint(cp);
  const b = enc.encode(s);
  console.log("cp_" + cp.toString(16) + "=units:" + s.length + " bytes:" + b.length + " [" + hex(b) + "] roundtrip:" + (dec.decode(b) === s));
}

const mixed = "A\u00e9\u20ac\u{1f600}";
console.log("mixed_units=" + mixed.length + " bytes=" + enc.encode(mixed).length + " [" + hex(enc.encode(mixed)) + "]");
console.log("mixed_codepoints=" + Array.from(mixed).length + " roundtrip=" + (dec.decode(enc.encode(mixed)) === mixed));
console.log("byte_length_matches=" + boundaries.map(function (cp) {
  const s = String.fromCodePoint(cp);
  return String(enc.encode(s).length === enc.encodeInto(s, new Uint8Array(8)).written);
}).join(","));

// Surrogate hole: a lone half becomes U+FFFD (ef bf bd), a valid pair one code point.
console.log("lone_high=" + hex(enc.encode("\ud83d")));
console.log("lone_low=" + hex(enc.encode("\ude00")));
console.log("lone_in_middle=" + hex(enc.encode("a\ud800b")));
console.log("two_highs=" + hex(enc.encode("\ud800\ud800")));
console.log("reversed_pair=" + hex(enc.encode("\ude00\ud83d")));
console.log("pair_then_lone=" + hex(enc.encode("\u{1f600}\ud800")));
console.log("pair=" + hex(enc.encode("\u{1f600}")));
console.log("replacement_itself=" + hex(enc.encode("\ufffd")));
console.log("lone_roundtrip=" + (function (): string {
  const back = dec.decode(enc.encode("\ud800"));
  return back.length + "/" + back.charCodeAt(0).toString(16);
})());
// A pair split across two encode() calls cannot be rejoined: the encoder is stateless.
const firstHalf = enc.encode("\ud83d");
const secondHalf = enc.encode("\ude00");
console.log("split_calls=" + hex(firstHalf) + " | " + hex(secondHalf) + " joined=" + hex(enc.encode("\u{1f600}")));

// encode() argument coercion and the shape of its answer.
console.log("encode_empty=" + enc.encode("").length + " no_arg=" + (enc as any).encode().length);
console.log("encode_undefined=" + hex(enc.encode(undefined as any)) + " null=" + hex(enc.encode(null as any)) + " number=" + hex(enc.encode(123 as any)));
console.log("encode_kind=" + enc.encode("A").constructor.name + " offset=" + enc.encode("A").byteOffset + " buffer=" + enc.encode("A").buffer.byteLength);
console.log("encode_fresh_each_time=" + (enc.encode("A") !== enc.encode("A")));
console.log("encode_ascii=" + hex(enc.encode("AB")) + " two_byte=" + hex(enc.encode("\u00e9")) + " three_byte=" + hex(enc.encode("\u20ac")));

// ---------------------------------------------------------------------------
// encodeInto: stops at the last WHOLE code point that fits; read counts UTF-16
// units, written counts bytes.
// ---------------------------------------------------------------------------
const into = function (s: string, size: number): string {
  const dest = new Uint8Array(size);
  const r = enc.encodeInto(s, dest);
  return r.read + "/" + r.written + "/[" + hex(dest) + "]";
};
console.log("into_exact_1=" + into("A", 1));
console.log("into_exact_2=" + into("\u00e9", 2));
console.log("into_short_2=" + into("\u00e9", 1));
console.log("into_exact_3=" + into("\u20ac", 3));
console.log("into_short_3=" + into("\u20ac", 2));
console.log("into_exact_4=" + into("\u{1f600}", 4));
console.log("into_short_4=" + into("\u{1f600}", 3));
console.log("into_ascii_then_astral=" + into("A\u{1f600}", 3));
console.log("into_ascii_then_astral_fits=" + into("A\u{1f600}", 5));
console.log("into_euro_pair=" + into("\u20ac\u20ac", 4));
console.log("into_zero_dest=" + into("A", 0));
console.log("into_empty_string=" + into("", 2));
console.log("into_lone_surrogate=" + into("\ud800x", 4));
console.log("into_lone_surrogate_short=" + into("\ud800x", 2));
console.log("into_read_is_units=" + (function (): string {
  const r = enc.encodeInto("\u{1f600}", new Uint8Array(4));
  return r.read + "/" + r.written;
})());
console.log("into_result_shape=" + (function (): string {
  const r: any = enc.encodeInto("A", new Uint8Array(4));
  return Object.keys(r).sort().join(",") + "/" + (Object.getPrototypeOf(r) === Object.prototype) + "/" + typeof r.read;
})());
console.log("into_leaves_tail=" + (function (): string {
  const dest = new Uint8Array(4).fill(0xaa);
  enc.encodeInto("A", dest);
  return hex(dest);
})());
console.log("into_view_window=" + (function (): string {
  const backing = new Uint8Array(8).fill(0xff);
  const r = enc.encodeInto("abc", backing.subarray(2, 6));
  return r.read + "/" + r.written + "/[" + hex(backing) + "]";
})());
console.log("into_lone_fits=" + (function (): string {
  const d3 = new Uint8Array(3);
  const a = enc.encodeInto("\ud800", d3);
  const d2 = new Uint8Array(2);
  const b = enc.encodeInto("\ud800", d2);
  return a.read + "/" + a.written + "/" + hex(d3) + " short:" + b.read + "/" + b.written + "/" + hex(d2);
})());

// ---------------------------------------------------------------------------
// Decoder, lossy mode: malformed sequences and how many U+FFFD they collapse to.
// ---------------------------------------------------------------------------
const lossy = new TextDecoder();
const fatal = new TextDecoder("utf-8", { fatal: true });
console.log("lossy_flags=" + lossy.encoding + " fatal=" + lossy.fatal + " ignoreBOM=" + lossy.ignoreBOM);
console.log("fatal_flags=" + fatal.encoding + " fatal=" + fatal.fatal + " ignoreBOM=" + fatal.ignoreBOM);

const cases: Array<[string, number[]]> = [
  ["overlong_2", [0xc0, 0x80]],
  ["overlong_3", [0xe0, 0x80, 0x80]],
  ["truncated_3", [0xe2, 0x82]],
  ["truncated_4", [0xf0, 0x9f, 0x98]],
  ["surrogate_ed", [0xed, 0xa0, 0x80]],
  ["lone_continuation", [0x80]],
  ["ff_byte", [0xff]],
  ["fe_byte", [0xfe]],
  ["above_max", [0xf5, 0x80, 0x80, 0x80]],
  ["c1_start", [0xc1, 0xbf]],
  ["bad_then_ascii", [0xff, 0x41]],
  ["truncated_then_ascii", [0xe2, 0x41]],
  ["valid_after_bad", [0x80, 0xc3, 0xa9]],
];
for (const c of cases) {
  const b = new Uint8Array(c[1]);
  const text = lossy.decode(b);
  let outcome = "no-throw";
  try {
    fatal.decode(b);
  } catch (e: any) {
    outcome = e.name;
  }
  console.log(c[0] + " lossy=" + cps(text) + " len=" + text.length + " fatal=" + outcome);
}
console.log("still_usable=" + cps(fatal.decode(bytes(0x41, 0xc3, 0xa9, 0xe2, 0x82, 0xac, 0xf0, 0x9f, 0x98, 0x80))));

// ---------------------------------------------------------------------------
// BOM: one leading EF BB BF is removed unless ignoreBOM; anywhere else it is U+FEFF.
// ---------------------------------------------------------------------------
const keeping = new TextDecoder("utf-8", { ignoreBOM: true });
const bomA = bytes(0xef, 0xbb, 0xbf, 0x41);
console.log("bom_stripped=" + units(lossy.decode(bomA)) + " kept=" + units(keeping.decode(bomA)));
console.log("length_difference=" + lossy.decode(bomA).length + "," + keeping.decode(bomA).length);
console.log("bom_only=" + JSON.stringify(lossy.decode(bytes(0xef, 0xbb, 0xbf))) + "," + units(keeping.decode(bytes(0xef, 0xbb, 0xbf))));
console.log("two_boms=" + units(lossy.decode(bytes(0xef, 0xbb, 0xbf, 0xef, 0xbb, 0xbf, 0x41))));
console.log("three_boms=" + units(lossy.decode(bytes(0xef, 0xbb, 0xbf, 0xef, 0xbb, 0xbf, 0xef, 0xbb, 0xbf))));
console.log("bom_in_middle=" + units(lossy.decode(bytes(0x41, 0xef, 0xbb, 0xbf, 0x42))));
console.log("bom_at_end=" + units(lossy.decode(bytes(0x41, 0xef, 0xbb, 0xbf))));
console.log("empty=" + JSON.stringify(lossy.decode(bytes())) + "," + JSON.stringify(keeping.decode(bytes())));
console.log("truncated_bom_two=" + units(lossy.decode(bytes(0xef, 0xbb))) + " one=" + units(lossy.decode(bytes(0xef))));
console.log("near_miss_bom=" + units(lossy.decode(bytes(0xef, 0xbb, 0xbe, 0x41))));
console.log("bom_under_fatal=" + units(new TextDecoder("utf-8", { fatal: true }).decode(bomA)));
console.log("bom_under_fatal_kept=" + units(new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bomA)));
console.log("second_call=" + (function (): string {
  const d = new TextDecoder();
  const first = units(d.decode(bytes(0xef, 0xbb, 0xbf, 0x41)));
  const second = units(d.decode(bytes(0xef, 0xbb, 0xbf, 0x42)));
  return first + "|" + second;
})());
console.log("bom_bytes_as_1252=" + units(new TextDecoder("windows-1252").decode(bytes(0xef, 0xbb, 0xbf, 0x41))));
console.log("encode_feff=" + hex(enc.encode("\ufeff")) + " feff_then_a=" + hex(enc.encode("\ufeffA")));
console.log("roundtrip_loses_bom=" + JSON.stringify(lossy.decode(enc.encode("\ufeffA"))));
console.log("roundtrip_keeps_bom=" + units(keeping.decode(enc.encode("\ufeffA"))));
console.log("encodeInto_feff=" + (function (): string {
  const dest = new Uint8Array(6);
  const r = enc.encodeInto("\ufeffA", dest);
  return r.read + "/" + r.written + "/" + hex(dest);
})());

// ---------------------------------------------------------------------------
// Fatal mode: truncated input throws; valid input decodes. Sources: ArrayBuffer,
// DataView, subarray, no argument.
// ---------------------------------------------------------------------------
console.log("truncated_fatal=" + t(function () { return new TextDecoder("utf-8", { fatal: true }).decode(bytes(0xef, 0xbb)); }));
console.log("no_arg=" + JSON.stringify(lossy.decode()));
console.log("from_buffer=" + cps(lossy.decode(enc.encode("A").buffer)));
console.log("from_dataview=" + cps(lossy.decode(new DataView(enc.encode("A").buffer))));
console.log("from_subarray=" + cps(lossy.decode(enc.encode("AB").subarray(1))));

// ---------------------------------------------------------------------------
// Streaming: an incomplete tail is held across calls, not replaced; without
// stream:true the same split produces replacements at the seam.
// ---------------------------------------------------------------------------
const whole = enc.encode("\u00e9\u20ac\u{1f600}!");
const streamer = new TextDecoder();
let assembled = "";
for (let i = 0; i < whole.length; i++) {
  assembled += streamer.decode(whole.subarray(i, i + 1), { stream: true });
}
assembled += streamer.decode();
console.log("stream_byte_by_byte=" + (assembled === "\u00e9\u20ac\u{1f600}!") + " len=" + assembled.length);

const halves = new TextDecoder();
console.log("stream_first_half=" + cps(halves.decode(whole.subarray(0, 4), { stream: true })));
console.log("stream_second_half=" + cps(halves.decode(whole.subarray(4), { stream: true })));
console.log("stream_flush=" + halves.decode().length);

const nonStreaming = new TextDecoder();
console.log("nostream_first=" + cps(nonStreaming.decode(whole.subarray(0, 4))));
console.log("nostream_second=" + cps(nonStreaming.decode(whole.subarray(4))));

const flushFatal = new TextDecoder("utf-8", { fatal: true });
console.log("held_tail=" + flushFatal.decode(enc.encode("\u20ac").subarray(0, 2), { stream: true }).length);
console.log("flush_incomplete=" + t(function () { return flushFatal.decode(); }));
const flushLossy = new TextDecoder();
console.log("flush_lossy_held=" + flushLossy.decode(enc.encode("\u20ac").subarray(0, 2), { stream: true }).length);
console.log("flush_lossy=" + cps(flushLossy.decode()));

// Digest of a TextEncoder output (async IIFE, no top-level await).
(async function () {
  const digest = await crypto.subtle.digest("SHA-256", enc.encode("abc"));
  console.log("sha256_8=" + hex(new Uint8Array(digest).subarray(0, 8)));
})();
