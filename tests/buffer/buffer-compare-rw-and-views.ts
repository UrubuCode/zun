// Buffer comparison ranges, partial UTF-8 writes, integer and float read/write
// (endianness, sign, bounds), toString / byteLength / concat / equals, slice vs
// subarray memory sharing, indexOf / includes family, and toJSON.

const t = (f: () => unknown): string => {
  try {
    return String(f());
  } catch (e: any) {
    return "throw:" + e.name + ":" + e.code;
  }
};

// ---- compare(target, targetStart, targetEnd, sourceStart, sourceEnd) ----
const A = Buffer.from("ABC");
const B = Buffer.from("BCD");
console.log("cmp_plain=" + t(() => A.compare(B)));
console.log("cmp_reverse=" + t(() => B.compare(A)));
console.log("cmp_equal=" + t(() => A.compare(Buffer.from("ABC"))));
console.log("cmp_ranges_eq=" + t(() => A.compare(B, 0, 1, 1, 2)));
console.log("cmp_ranges_src_gt=" + t(() => A.compare(B, 1, 3, 0, 2)));
console.log("cmp_empty_ranges=" + t(() => A.compare(B, 3, 3, 3, 3)));
console.log("cmp_zero_target=" + t(() => A.compare(B, 0, 0, 0, 3)));
console.log("cmp_start_gt_end=" + t(() => A.compare(B, 2, 1)));
console.log("cmp_reversed_src=" + t(() => A.compare(B, 0, 3, 2, 1)));
console.log("cmp_neg_target_start=" + t(() => A.compare(B, -1)));
console.log("cmp_start_past_len=" + t(() => A.compare(B, 4)));
console.log("cmp_end_past_len=" + t(() => A.compare(B, 0, 4)));
console.log("cmp_src_end_past=" + t(() => A.compare(B, 0, 3, 4)));
console.log("cmp_src_end_past2=" + t(() => A.compare(B, 0, 3, 0, 4)));
console.log("cmp_neg_end=" + t(() => A.compare(B, 0, -1)));
console.log("cmp_undef_mix=" + t(() => A.compare(B, undefined, undefined, 1)));
console.log("cmp_frac_start=" + t(() => A.compare(B, 1.5)));
console.log("cmp_string_start=" + t(() => A.compare(B, "1" as any)));
console.log("cmp_undef_end=" + t(() => A.compare(B, 0, 3, 0, undefined)));
console.log("cmp_undef_tend=" + t(() => A.compare(B, 0, undefined, 1, 2)));
console.log("cmp_start_gt_end2=" + t(() => A.compare(B, 3, 2)));
console.log("cmp_src_rev2=" + t(() => A.compare(B, 0, 3, 3, 2)));
console.log("cmp_nan_start=" + t(() => A.compare(B, NaN)));
console.log("cmp_src_one=" + t(() => A.compare(B, 0, 2, 0, 1)));
console.log("cmp_neg_src_start=" + t(() => A.compare(B, 0, 3, -1)));
console.log("cmp_frac_src=" + t(() => A.compare(B, 0, 3, 0.5)));
console.log("cmp_string_src=" + t(() => A.compare(B, 0, 3, "1" as any)));
console.log("cmp_string_tend=" + t(() => A.compare(B, 0, "1" as any)));
console.log("cmp_frac_send=" + t(() => A.compare(B, 0, 3, 0, 1.5)));
console.log("cmp_nan_send=" + t(() => A.compare(B, 0, 3, 0, NaN)));
console.log("cmp_null_start=" + t(() => A.compare(B, null as any)));
console.log("cmp_null_tend=" + t(() => A.compare(B, 0, null as any)));
console.log("cmp_inf_start=" + t(() => A.compare(B, Infinity)));
console.log("cmp_big_start=" + t(() => A.compare(B, 2 ** 53)));
console.log("cmp_big_send=" + t(() => A.compare(B, 0, 3, 2 ** 53)));
console.log("cmp_neg_both=" + t(() => A.compare(B, -1, -1)));
console.log("cmp_zero_src=" + t(() => A.compare(B, 0, 3, 0, 0)));
console.log("cmp_bad_target=" + t(() => A.compare(5 as any)));

// ---- equals / lexical compare sort ----
console.log("equals_true=" + Buffer.from("xy").equals(Buffer.from([120, 121])));
console.log("equals_false=" + Buffer.from("xy").equals(Buffer.from("xz")));
console.log("equals_len=" + Buffer.from("x").equals(Buffer.from("xx")));
const sorted = [Buffer.from("b"), Buffer.from("ab"), Buffer.from("a")].sort(Buffer.compare);
console.log("sort_buffers=" + sorted.map((b) => b.toString()).join(","));

// ---- write(): partial UTF-8 writes never split a code point ----
for (const n of [0, 1, 2, 3, 4, 5]) {
  const b = Buffer.alloc(n);
  const r = b.write("€");
  console.log("write_euro_len" + n + "=" + r + " " + b.toString("hex"));
}
{ const b = Buffer.alloc(2); console.log("write_a_euro=" + b.write("a€") + " " + b.toString("hex")); }
{ const b = Buffer.alloc(4); console.log("write_a_euro2=" + b.write("a€€") + " " + b.toString("hex")); }
{ const b = Buffer.alloc(6); console.log("write_offset_emoji=" + b.write("x😀", 1) + " " + b.toString("hex")); }
{ const b = Buffer.alloc(8); console.log("write_maxlen_emoji=" + b.write("😀😀", 0, 6) + " " + b.toString("hex")); }
{ const b = Buffer.alloc(8); console.log("write_maxlen_mixed=" + b.write("é€", 0, 2, "utf8") + " " + b.toString("hex")); }
{ const b = Buffer.alloc(8); console.log("utf8Write_maxlen=" + b.utf8Write("€€", 0, 5) + " " + b.toString("hex")); }
{ const b = Buffer.alloc(3); console.log("write_lone_surrogate=" + b.write("\ud800ab") + " " + b.toString("hex")); }
{ const b = Buffer.alloc(2); console.log("write_lone_only=" + b.write("\ud800") + " " + b.toString("hex")); }
{ const b = Buffer.alloc(4); console.log("write_hex_partial=" + b.write("abcdef", 1, "hex") + " " + b.toString("hex")); }
{ const b = Buffer.alloc(4); console.log("write_latin1=" + b.write("é€", 0, "latin1") + " " + b.toString("hex")); }
{ const b = Buffer.alloc(4); console.log("write_bad_offset=" + t(() => b.write("a", 9))); }

// ---- integer / float read & write (endianness, sign, bounds) ----
{
  const b = Buffer.alloc(16);
  console.log("writeUInt8=" + b.writeUInt8(0xfe, 0));
  console.log("writeInt8=" + b.writeInt8(-2, 1));
  console.log("writeUInt16BE=" + b.writeUInt16BE(0x1234, 2));
  console.log("writeUInt16LE=" + b.writeUInt16LE(0x1234, 4));
  console.log("writeInt16LE=" + b.writeInt16LE(-300, 6));
  console.log("writeUInt32LE=" + b.writeUInt32LE(0xdeadbeef, 8));
  console.log("writeInt32BE=" + b.writeInt32BE(-2, 12));
  console.log("bytes_hex=" + b.toString("hex"));
  console.log("readUInt8=" + b.readUInt8(0) + " readInt8=" + b.readInt8(0) + " readInt8b=" + b.readInt8(1));
  console.log("readUInt16BE=" + b.readUInt16BE(2) + " readUInt16LE=" + b.readUInt16LE(4));
  console.log("readInt16LE=" + b.readInt16LE(6) + " readInt16BE=" + b.readInt16BE(6));
  console.log("readUInt32LE=" + b.readUInt32LE(8) + " readInt32LE=" + b.readInt32LE(8));
  console.log("readUInt32BE=" + b.readUInt32BE(8));
  console.log("readInt32BE=" + b.readInt32BE(12));
  console.log("readUIntBE3=" + b.readUIntBE(8, 3) + " readUIntLE3=" + b.readUIntLE(8, 3));
  console.log("readIntBE6=" + b.readIntBE(6, 6));
  console.log("readUInt8_oob=" + t(() => b.readUInt8(16)));
  console.log("readUInt32_oob=" + t(() => b.readUInt32LE(13)));
  console.log("readUInt8_neg=" + t(() => b.readUInt8(-1)));
  console.log("readUInt8_frac=" + t(() => b.readUInt8(1.5)));
  console.log("writeUInt8_overflow=" + t(() => b.writeUInt8(256, 0)));
  console.log("writeInt8_underflow=" + t(() => b.writeInt8(-129, 0)));
  console.log("writeUInt16_bad=" + t(() => b.writeUInt16LE(0x10000, 0)));
  console.log("writeUInt32_oob=" + t(() => b.writeUInt32BE(1, 13)));
  console.log("bytes_after_fail=" + b.toString("hex"));
}
{
  const b = Buffer.alloc(8);
  b.writeDoubleLE(1.5, 0);
  console.log("double_le=" + b.toString("hex") + " back=" + b.readDoubleLE(0));
  b.writeDoubleBE(-0.25, 0);
  console.log("double_be=" + b.toString("hex") + " back=" + b.readDoubleBE(0));
  b.writeFloatBE(3.5, 0);
  console.log("float_be=" + b.toString("hex").slice(0, 8) + " back=" + b.readFloatBE(0));
  b.writeFloatLE(0.1, 4);
  console.log("float_le_0.1=" + b.readFloatLE(4));
  b.writeBigUInt64LE(0x0102030405060708n, 0);
  console.log("bigu64le=" + b.toString("hex") + " back=" + String(b.readBigUInt64LE(0)));
  b.writeBigInt64BE(-2n, 0);
  console.log("bigi64be=" + b.toString("hex") + " back=" + String(b.readBigInt64BE(0)));
  console.log("bigu64_oob=" + t(() => b.readBigUInt64LE(1)));
}

// ---- toString encodings and ranges ----
{
  const b = Buffer.from("héllo wörld", "utf8");
  console.log("toString_utf8=" + b.toString());
  console.log("toString_hex=" + b.toString("hex"));
  console.log("toString_base64=" + b.toString("base64"));
  console.log("toString_base64url=" + Buffer.from([0xfb, 0xff]).toString("base64url"));
  console.log("toString_latin1=" + Buffer.from("é").toString("latin1").length);
  console.log("toString_ascii_hi=" + Buffer.from([0x41, 0xe9]).toString("ascii").charCodeAt(1));
  console.log("toString_utf16le=" + Buffer.from("hi", "utf16le").toString("utf16le"));
  console.log("toString_range=" + b.toString("utf8", 1, 3));
  console.log("toString_range_hex=" + b.toString("hex", 0, 2));
  console.log("toString_neg_end=" + JSON.stringify(b.toString("utf8", 0, -1)));
  console.log("toString_start_gt_end=" + JSON.stringify(b.toString("utf8", 5, 2)));
  console.log("toString_bad_enc=" + t(() => b.toString("nope" as any)));
  console.log("toJSON=" + JSON.stringify(Buffer.from([1, 255])));
  console.log("toJSON_type=" + Buffer.from([1]).toJSON().type);
}

// ---- byteLength / concat ----
console.log("byteLength_utf8=" + Buffer.byteLength("€"));
console.log("byteLength_emoji=" + Buffer.byteLength("😀"));
console.log("byteLength_hex=" + Buffer.byteLength("abcd", "hex"));
console.log("byteLength_base64=" + Buffer.byteLength("aGVsbG8=", "base64"));
console.log("byteLength_latin1=" + Buffer.byteLength("é€", "latin1"));
console.log("byteLength_utf16=" + Buffer.byteLength("ab", "utf16le"));
console.log("byteLength_buf=" + Buffer.byteLength(Buffer.alloc(7)));
console.log("byteLength_ab=" + Buffer.byteLength(new ArrayBuffer(5)));
console.log("concat=" + Buffer.concat([Buffer.from("ab"), Buffer.from([0x63]), Buffer.from("")]).toString());
console.log("concat_total_trim=" + Buffer.concat([Buffer.from("abc"), Buffer.from("de")], 4).toString());
console.log("concat_total_pad=" + Buffer.concat([Buffer.from("a")], 3).toString("hex"));
console.log("concat_empty=" + Buffer.concat([]).length);
console.log("concat_bad=" + t(() => Buffer.concat(["a"] as any)));

// ---- slice vs subarray: memory sharing and negative indices ----
{
  const base = Buffer.from("abcdef");
  const sl = base.slice(1, 4);
  const sa = base.subarray(-3);
  sl[0] = 0x58;
  sa[0] = 0x59;
  console.log("slice_shares=" + base.toString() + " " + sl.toString() + " " + sa.toString());
  console.log("slice_neg=" + base.slice(-2).toString() + " " + base.slice(4, 1).length);
  console.log("subarray_clamped=" + base.subarray(2, 100).toString());
  const copy = Buffer.from(base);
  copy[0] = 0x5a;
  console.log("from_copies=" + base[0] + " " + copy[0]);
}

// ---- indexOf / lastIndexOf / includes ----
{
  const h = Buffer.from("abcabcab");
  console.log("indexOf_str=" + h.indexOf("bc"));
  console.log("indexOf_from=" + h.indexOf("bc", 2));
  console.log("indexOf_neg_from=" + h.indexOf("ab", -3));
  console.log("indexOf_missing=" + h.indexOf("zz"));
  console.log("indexOf_num=" + h.indexOf(0x63));
  console.log("indexOf_buf=" + h.indexOf(Buffer.from("ca")));
  console.log("indexOf_empty=" + h.indexOf(""));
  console.log("lastIndexOf_str=" + h.lastIndexOf("ab"));
  console.log("lastIndexOf_from=" + h.lastIndexOf("ab", 4));
  console.log("lastIndexOf_neg=" + h.lastIndexOf("ab", -5));
  console.log("includes_true=" + h.includes("cab"));
  console.log("includes_false=" + h.includes("cc"));
  console.log("indexOf_hex=" + h.indexOf("6263", 0, "hex"));
  console.log("indexOf_latin1=" + Buffer.from("aé").indexOf("é", 0, "latin1"));
  console.log("indexOf_fromEnd=" + h.indexOf("a", 100));
}

// ---- swap / misc ----
console.log("swap16=" + Buffer.from([1, 2, 3, 4]).swap16().toString("hex"));
console.log("swap32=" + Buffer.from([1, 2, 3, 4]).swap32().toString("hex"));
console.log("swap16_odd=" + t(() => Buffer.from([1, 2, 3]).swap16()));
console.log("reverse=" + Buffer.from("abc").reverse().toString());
console.log("iterate=" + [...Buffer.from("hi")].join(","));
console.log("json_roundtrip_hex=" + Buffer.from(JSON.parse(JSON.stringify(Buffer.from([9, 8])))).toString("hex"));
