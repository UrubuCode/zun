// Buffer construction and fill semantics: Buffer.from over strings (utf8/hex/base64/
// utf16le/latin1), ArrayBuffer views, arrays (ToUint8 modulo wrap), array-likes,
// valueOf / Symbol.toPrimitive / JSON precedence, Buffer.alloc / allocUnsafe lengths
// (contents of allocUnsafe are never printed), and Buffer#fill argument forms.

const show = (v: any): string => (Buffer.isBuffer(v) ? "buf:" + v.toString("hex") : String(v));
const t = (f: () => any): string => {
  try {
    return show(f());
  } catch (e: any) {
    return "throw:" + e.name + ":" + e.code;
  }
};

// ---- Buffer.from(string, encoding) ----
console.log("utf8=" + t(() => Buffer.from("héllo€", "utf8").toString("hex")));
console.log("utf8_default=" + t(() => Buffer.from("€").toString("hex")));
console.log("utf8_len=" + Buffer.from("€😀").length);
console.log("hex_pairs=" + t(() => Buffer.from("48656c6C6f", "hex").toString("utf8")));
console.log("hex_odd_truncates=" + t(() => Buffer.from("abc", "hex").toString("hex")));
console.log("hex_invalid_stops=" + t(() => Buffer.from("zz41", "hex").toString("hex")));
console.log("hex_mid_invalid=" + t(() => Buffer.from("41zz42", "hex").toString("hex")));
console.log("base64=" + t(() => Buffer.from("aGVsbG8=", "base64").toString("utf8")));
console.log("base64url=" + t(() => Buffer.from("-_8", "base64url").toString("hex")));
console.log("base64_no_pad=" + t(() => Buffer.from("aGk", "base64").toString("utf8")));
console.log("base64_ignores_ws=" + t(() => Buffer.from("aG Vs\nbG8=", "base64").toString("utf8")));
console.log("latin1=" + t(() => Buffer.from("é€", "latin1").toString("hex")));
console.log("ascii_high_bit=" + t(() => Buffer.from("é", "ascii").toString("hex")));
console.log("utf16le=" + t(() => Buffer.from("ab€", "utf16le").toString("hex")));
console.log("ucs2_alias=" + t(() => Buffer.from("a", "ucs2").toString("hex")));
console.log("unknown_encoding=" + t(() => Buffer.from("a", "nope" as any).toString("hex")));
console.log("case_insensitive_enc=" + t(() => Buffer.from("ab", "HEX").toString("hex")));
console.log("empty_string=" + t(() => Buffer.from("").length));
console.log("bad_type_number=" + t(() => Buffer.from(5 as any)));
console.log("bad_type_undefined=" + t(() => Buffer.from(undefined as any)));

// ---- Buffer.from(ArrayBuffer / views / arrays) ----
const ab = new ArrayBuffer(4);
const u8 = new Uint8Array(ab);
u8.set([1, 2, 3, 4]);
const shared = Buffer.from(ab);
shared[0] = 99;
console.log("arraybuffer_shares=" + u8[0] + " " + shared.length);
console.log("arraybuffer_offset=" + t(() => Buffer.from(ab, 1, 2).toString("hex")));
console.log("arraybuffer_offset_default=" + t(() => Buffer.from(ab, 2).toString("hex")));
console.log("arraybuffer_oob=" + t(() => Buffer.from(ab, 5).toString("hex")));
console.log("u16_view_copies=" + t(() => Buffer.from(new Uint16Array([0x0102, 0x0304])).toString("hex")));
console.log("u8_view_copies=" + t(() => Buffer.from(new Uint8Array([7, 8])).toString("hex")));
console.log("array_wrap=" + t(() => Buffer.from([1, 257, -1, 3.7, "5", NaN, 300, -300, 2 ** 32 + 7]).toString("hex")));
console.log("array_empty=" + t(() => Buffer.from([]).length));
console.log("array_nested_string=" + t(() => Buffer.from(["7", "x"] as any).toString("hex")));

// ---- Buffer.from(object): precedence and array-like form ----
const hex = (b: any) => b.toString("hex");
const objT = (f: () => any): string => {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.name + ":" + e.code;
  }
};
console.log("arraylike=" + objT(() => hex(Buffer.from({ length: 2, 0: 5, 1: 6 } as any))));
console.log("missing_keys=" + objT(() => hex(Buffer.from({ length: 3, 0: 1 } as any))));
console.log("length_zero=" + objT(() => hex(Buffer.from({ length: 0 } as any))));
console.log("length_negative=" + objT(() => hex(Buffer.from({ length: -1 } as any))));
console.log("length_string_is_empty=" + objT(() => hex(Buffer.from({ length: "2", 0: 7 } as any))));
console.log("length_nan=" + objT(() => hex(Buffer.from({ length: NaN, 0: 7 } as any))));
console.log("no_length=" + objT(() => hex(Buffer.from({ a: 1 } as any))));
console.log("wrap=" + objT(() => hex(Buffer.from({ length: 4, 0: 257, 1: -1, 2: 1.9, 3: "7" } as any))));
console.log("non_finite=" + objT(() => hex(Buffer.from({ length: 3, 0: NaN, 1: Infinity, 2: "x" } as any))));
const order: string[] = [];
const getters: any = { length: 3 };
for (let i = 0; i < 3; i++) {
  Object.defineProperty(getters, i, {
    get: function () { order.push("g" + i); return i + 1; },
    enumerable: true,
  });
}
console.log("getters=" + objT(() => hex(Buffer.from(getters))) + " " + order.join(">"));
console.log("valueOf_string=" + objT(() => hex(Buffer.from({ valueOf() { return "hi"; } } as any))));
console.log("valueOf_beats_arraylike=" + objT(() => hex(Buffer.from({ valueOf() { return "hi"; }, length: 1, 0: 9 } as any))));
console.log("valueOf_object=" + objT(() => hex(Buffer.from({ valueOf() { return { length: 1, 0: 9 }; } } as any))));
console.log("valueOf_hex_encoding=" + objT(() => hex(Buffer.from({ valueOf() { return "6869"; } } as any, "hex"))));
console.log("valueOf_number_refused=" + objT(() => hex(Buffer.from({ valueOf() { return 5; } } as any))));
console.log("valueOf_number_then_arraylike=" + objT(() => hex(Buffer.from({ valueOf() { return 5; }, length: 1, 0: 3 } as any))));
console.log("valueOf_throws=" + objT(() => hex(Buffer.from({ valueOf() { throw new RangeError("boom"); } } as any))));
console.log("toPrimitive_string=" + objT(() => hex(Buffer.from({ [Symbol.toPrimitive]() { return "ok"; } } as any))));
console.log("arraylike_beats_toPrimitive=" + objT(() => hex(Buffer.from({ length: 1, 0: 8, [Symbol.toPrimitive]() { return "ok"; } } as any))));
console.log("toPrimitive_number_refused=" + objT(() => hex(Buffer.from({ [Symbol.toPrimitive]() { return 1; } } as any))));
console.log("json_form=" + objT(() => hex(Buffer.from({ type: "Buffer", data: [1, 2, 3] } as any))));
console.log("json_roundtrip=" + objT(() => hex(Buffer.from(JSON.parse(JSON.stringify(Buffer.from("abc")))))));
console.log("empty_object_refused=" + objT(() => hex(Buffer.from({} as any))));

// ---- Buffer.alloc / allocUnsafe: lengths only, allocUnsafe contents never printed ----
console.log("alloc_zero_fill=" + Buffer.alloc(4).toString("hex"));
console.log("alloc_fill_num=" + Buffer.alloc(3, 7).toString("hex"));
console.log("alloc_fill_str=" + Buffer.alloc(5, "ab").toString("hex"));
console.log("alloc_fill_hex=" + Buffer.alloc(4, "6162", "hex").toString("hex"));
console.log("alloc_fill_empty=" + Buffer.alloc(3, "").toString("hex"));
console.log("alloc_neg_throws=" + t(() => Buffer.alloc(-1)));
console.log("alloc_zero_len=" + Buffer.alloc(0).length);
console.log("allocUnsafe_len=" + Buffer.allocUnsafe(6).length);
console.log("allocUnsafe_isBuffer=" + Buffer.isBuffer(Buffer.allocUnsafe(2)));
console.log("allocUnsafe_writable=" + (() => { const b = Buffer.allocUnsafe(2); b[0] = 0xab; b[1] = 0xcd; return b.toString("hex"); })());
console.log("isBuffer_uint8=" + Buffer.isBuffer(new Uint8Array(2)));
console.log("isEncoding_utf8=" + Buffer.isEncoding("utf8") + " " + Buffer.isEncoding("nope"));

// ---- Buffer#fill argument forms ----
console.log("fill_range_clamped=" + t(() => Buffer.alloc(3).fill(1, 0, 10)));
console.log("fill_neg_offset=" + t(() => Buffer.alloc(3).fill(1, -1)));
console.log("fill_offset_past_end=" + t(() => Buffer.alloc(3).fill(1, 4)));
console.log("fill_offset_str=" + t(() => Buffer.alloc(3).fill(1, "a")));
console.log("fill_end_str=" + t(() => Buffer.alloc(3).fill(1, 0, "a")));
console.log("fill_hex_invalid=" + t(() => Buffer.alloc(3).fill("zz", "hex")));
console.log("fill_empty_buf=" + t(() => Buffer.alloc(3).fill(Buffer.alloc(0))));
console.log("fill_bad_enc_slot=" + t(() => Buffer.alloc(3).fill("a", "nope")));
console.log("fill_bad_enc_last=" + t(() => Buffer.alloc(3).fill("a", 0, 2, "nope")));
console.log("fill_end_frac=" + t(() => Buffer.alloc(3).fill("a", 0, 1.5)));
console.log("fill_offset_frac=" + t(() => Buffer.alloc(3).fill("a", 1.5)));
console.log("fill_offset_nan=" + t(() => Buffer.alloc(3).fill("a", NaN)));
console.log("fill_end_nan=" + t(() => Buffer.alloc(3).fill("a", 0, NaN)));
console.log("fill_offset_inf=" + t(() => Buffer.alloc(3).fill("a", Infinity)));
console.log("fill_enc_in_slot=" + t(() => Buffer.alloc(3).fill("a", 0, undefined, "hex")));
console.log("fill_enc_undef_slots=" + t(() => Buffer.alloc(3).fill("a", undefined, undefined, "hex")));
console.log("fill_hex_last=" + t(() => Buffer.alloc(3).fill("6162", undefined, undefined, "hex")));
console.log("fill_num_bad_enc=" + t(() => Buffer.alloc(3).fill(1, 0, 2, "nope")));
console.log("fill_latin1_range=" + t(() => Buffer.alloc(4).fill("é", 1, 3)));
console.log("fill_latin1_enc=" + t(() => Buffer.alloc(4).fill("é", "latin1")));
console.log("fill_object=" + t(() => Buffer.alloc(4).fill({} as any)));
console.log("fill_null=" + t(() => Buffer.alloc(4).fill(null as any)));
console.log("fill_undefined=" + t(() => Buffer.alloc(4).fill(undefined as any)));
console.log("fill_reversed_range=" + t(() => Buffer.alloc(4).fill("a", 3, 2)));
console.log("fill_pattern_repeat=" + t(() => Buffer.alloc(8).fill("abc")));
console.log("fill_pattern_window=" + t(() => Buffer.alloc(6).fill("abc", 1, 5)));
console.log("fill_hex_pattern=" + t(() => Buffer.alloc(5).fill("6162", "hex")));
console.log("fill_hex_offset_slot=" + t(() => Buffer.alloc(6).fill("6162", 1, "hex")));
console.log("fill_hex_window=" + t(() => Buffer.alloc(6).fill("6162", 1, 4, "hex")));
console.log("fill_utf16le=" + t(() => Buffer.alloc(6).fill("ab", "utf16le")));
console.log("fill_base64=" + t(() => Buffer.alloc(6).fill("aGk=", "base64")));
console.log("fill_buf_pattern=" + t(() => Buffer.alloc(5).fill(Buffer.from([1, 2]))));
console.log("fill_wrap_257=" + t(() => Buffer.alloc(4).fill(0x101, 1)));
console.log("fill_neg_value=" + t(() => Buffer.alloc(3).fill(-1)));
