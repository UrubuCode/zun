// TextDecoder construction: label normalisation and aliases, the constructor's
// RangeError for unmapped labels, option flags and their getters, and the UTF-16
// decoders (odd bytes, surrogates, BOM, streaming). Every line is label=value.

const units = function (s: string): string {
  const out: string[] = [];
  for (let i = 0; i < s.length; i++) out.push(s.charCodeAt(i).toString(16));
  return out.join(",");
};
const t = function (f: () => any): string {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.name;
  }
};
const labelOf = function (label: any): string {
  return t(function () { return new TextDecoder(label).encoding; });
};
const bytesOf = function (...v: number[]): Uint8Array {
  return new Uint8Array(v);
};

// ---------------------------------------------------------------------------
// Label aliases: several spellings answer one canonical .encoding.
// ---------------------------------------------------------------------------
for (const label of ["utf-8", "UTF-8", "utf8", "UTF8", "unicode-1-1-utf-8", "unicode11utf8", "unicode20utf8", "x-unicode20utf8"]) {
  console.log("utf8 " + label + "=" + labelOf(label));
}
// latin1 and ascii are deliberate ALIASES of windows-1252 in the Encoding Standard.
for (const label of ["windows-1252", "latin1", "iso-8859-1", "iso8859-1", "ascii", "us-ascii", "cp1252", "ansi_x3.4-1968"]) {
  console.log("single " + label + "=" + labelOf(label));
}
for (const label of ["utf-16le", "utf-16", "ucs-2", "unicode", "utf-16be", "big5", "gbk", "gb18030", "shift_jis", "sjis", "euc-jp", "euc-kr", "iso-2022-jp"]) {
  console.log("other " + label + "=" + labelOf(label));
}

// Normalisation: ASCII whitespace around the label is stripped and the case folded;
// nothing inside the label is touched and NBSP is not whitespace.
console.log("trim_spaces=" + labelOf("  utf-8  "));
console.log("trim_tabs=" + labelOf("\tutf-8\n"));
console.log("trim_all_ws=" + labelOf("\n\r\t\f utf-8 \f\t\r\n"));
console.log("mixed_case=" + labelOf("UtF-8"));
console.log("inner_space=" + labelOf("utf 8"));
console.log("inner_dash_missing=" + labelOf("utf16le"));
console.log("nbsp_not_trimmed=" + labelOf("\u00a0utf-8"));

// No label versus an empty one.
console.log("no_argument=" + new TextDecoder().encoding);
console.log("undefined_label=" + labelOf(undefined));
console.log("empty_label=" + labelOf(""));
console.log("whitespace_only=" + labelOf("   "));

// Unmapped labels are refused by the CONSTRUCTOR, not at decode time.
console.log("bogus=" + labelOf("utf-9"));
console.log("almost=" + labelOf("utf-8x"));
console.log("bom_label=" + labelOf("utf-8-bom"));
console.log("nonstandard=" + labelOf("utf-32") + "," + labelOf("utf-32le") + "," + labelOf("ucs-4"));

// ---------------------------------------------------------------------------
// Options bag: only fatal and ignoreBOM are read, and they coerce as booleans.
// ---------------------------------------------------------------------------
console.log("flags_default=" + (function (): string { const d = new TextDecoder("utf-8"); return d.fatal + "," + d.ignoreBOM; })());
console.log("flags_set=" + (function (): string { const d = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }); return d.fatal + "," + d.ignoreBOM; })());
console.log("flags_empty_bag=" + t(function () { const d = new TextDecoder("utf-8", {} as any); return d.fatal + "," + d.ignoreBOM; }));
console.log("flags_extra_key=" + t(function () { const d = new TextDecoder("utf-8", { nope: 1 } as any); return d.fatal + "," + d.ignoreBOM; }));
console.log("flags_truthy=" + (function (): string { const d = new TextDecoder("utf-8", { fatal: 1 as any, ignoreBOM: "" as any }); return d.fatal + "," + d.ignoreBOM; })());
console.log("flags_are_getters=" + (function (): string {
  const d: any = Object.getOwnPropertyDescriptor(TextDecoder.prototype, "fatal");
  const e: any = Object.getOwnPropertyDescriptor(TextDecoder.prototype, "encoding");
  return typeof d.get + "," + typeof e.get + "," + String(Object.getOwnPropertyDescriptor(new TextDecoder(), "encoding"));
})());
console.log("props=" + (function (): string {
  const d = new TextDecoder("utf-16be", { fatal: true, ignoreBOM: true });
  return d.encoding + "," + d.fatal + "," + d.ignoreBOM;
})());
console.log("tags=" + Object.prototype.toString.call(new TextDecoder()) + "," + Object.prototype.toString.call(new TextEncoder()));
console.log("no_new=" + t(function () { return (TextDecoder as any)("utf-8"); }));

// TextEncoder has exactly one encoding and ignores any label.
console.log("encoder_encoding=" + new TextEncoder().encoding);
console.log("encoder_ignores_label=" + t(function () { return new (TextEncoder as any)("utf-16le").encoding; }));
console.log("decode_sources=" + t(function () { return new TextDecoder().decode(new Uint8Array([65]).buffer); }) + "," + t(function () { return new TextDecoder().decode(new DataView(new Uint8Array([66]).buffer)); }) + "," + t(function () { return new TextDecoder().decode([65] as any); }));

// ---------------------------------------------------------------------------
// UTF-16: bytes are read in pairs; an odd trailing byte becomes U+FFFD (or throws
// when fatal); an unpaired surrogate is replaced even under fatal; the BOM is
// consumed unless ignoreBOM is set.
// ---------------------------------------------------------------------------
const le = new TextDecoder("utf-16le");
const be = new TextDecoder("utf-16be");
console.log("le_ascii=" + le.decode(bytesOf(0x41, 0x00, 0x42, 0x00)));
console.log("be_ascii=" + be.decode(bytesOf(0x00, 0x41, 0x00, 0x42)));
console.log("le_of_be_bytes=" + units(le.decode(bytesOf(0x00, 0x41))));
console.log("le_units=" + units(le.decode(bytesOf(0xac, 0x20))));
console.log("be_units=" + units(be.decode(bytesOf(0x20, 0xac))));
console.log("le_empty=" + JSON.stringify(le.decode(new Uint8Array(0))) + " no_arg=" + JSON.stringify(le.decode()));

console.log("odd_tail=" + units(le.decode(bytesOf(0x41, 0x00, 0x42))));
console.log("odd_only=" + units(le.decode(bytesOf(0x41))));
console.log("odd_fatal=" + t(function () { return new TextDecoder("utf-16le", { fatal: true }).decode(bytesOf(0x41, 0x00, 0x42)); }));
console.log("even_fatal=" + t(function () { return new TextDecoder("utf-16le", { fatal: true }).decode(bytesOf(0x41, 0x00)); }));

console.log("pair=" + units(le.decode(bytesOf(0x3d, 0xd8, 0x00, 0xde))) + " codepoint=" + (le.decode(bytesOf(0x3d, 0xd8, 0x00, 0xde)).codePointAt(0) as number).toString(16));
console.log("lone_high=" + units(le.decode(bytesOf(0x00, 0xd8, 0x41, 0x00))));
console.log("lone_low=" + units(le.decode(bytesOf(0x00, 0xdc, 0x41, 0x00))));
console.log("reversed_pair=" + units(le.decode(bytesOf(0x00, 0xde, 0x3d, 0xd8))));
console.log("high_at_end=" + units(le.decode(bytesOf(0x41, 0x00, 0x00, 0xd8))));
console.log("lone_fatal=" + t(function () { return new TextDecoder("utf-16le", { fatal: true }).decode(bytesOf(0x00, 0xd8, 0x41, 0x00)); }));
console.log("pair_fatal=" + t(function () { return units(new TextDecoder("utf-16le", { fatal: true }).decode(bytesOf(0x3d, 0xd8, 0x00, 0xde))); }));

console.log("bom_stripped=" + units(new TextDecoder("utf-16le").decode(bytesOf(0xff, 0xfe, 0x41, 0x00))));
console.log("bom_kept=" + units(new TextDecoder("utf-16le", { ignoreBOM: true }).decode(bytesOf(0xff, 0xfe, 0x41, 0x00))));
console.log("be_bom_stripped=" + units(new TextDecoder("utf-16be").decode(bytesOf(0xfe, 0xff, 0x00, 0x41))));
console.log("wrong_bom_for_le=" + units(new TextDecoder("utf-16le").decode(bytesOf(0xfe, 0xff, 0x41, 0x00))));
console.log("bom_mid=" + units(new TextDecoder("utf-16le").decode(bytesOf(0x41, 0x00, 0xff, 0xfe, 0x42, 0x00))));
console.log("bom_only=" + units(new TextDecoder("utf-16le").decode(bytesOf(0xff, 0xfe))));

// Streaming: a half unit or half surrogate pair is carried across calls.
console.log("stream_split_unit=" + (function (): string {
  const d = new TextDecoder("utf-16le");
  const a = units(d.decode(bytesOf(0x41), { stream: true }));
  const b = units(d.decode(bytesOf(0x00), { stream: true }));
  const c = units(d.decode());
  return JSON.stringify(a) + "|" + b + "|" + JSON.stringify(c);
})());
console.log("stream_split_pair=" + (function (): string {
  const d = new TextDecoder("utf-16le");
  const a = units(d.decode(bytesOf(0x3d, 0xd8), { stream: true }));
  const b = units(d.decode(bytesOf(0x00, 0xde), { stream: true }));
  return JSON.stringify(a) + "|" + b;
})());
console.log("stream_flush_odd=" + (function (): string {
  const d = new TextDecoder("utf-16le");
  const a = units(d.decode(bytesOf(0x41, 0x00, 0x42), { stream: true }));
  const b = units(d.decode());
  return a + "|" + b;
})());
console.log("stream_flush_dangling_high=" + (function (): string {
  const d = new TextDecoder("utf-16le");
  const a = units(d.decode(bytesOf(0x3d, 0xd8), { stream: true }));
  const b = units(d.decode());
  return JSON.stringify(a) + "|" + b;
})());
console.log("decoder_state_resets=" + (function (): string {
  const d = new TextDecoder("utf-16le");
  d.decode(bytesOf(0x41), { stream: true });
  d.decode();
  return units(d.decode(bytesOf(0x42, 0x00)));
})());
