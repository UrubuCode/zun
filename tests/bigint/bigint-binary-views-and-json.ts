// Consolidated BigInt fixture: the 64-bit binary views (DataView and
// BigInt64Array/BigUint64Array over one buffer, both endiannesses), the
// wrapping and refusal rules of those entry points, JSON.stringify refusing
// BigInt with the replacer and toJSON escape hatches, JSON.parse precision
// loss, and the default sort comparing decimal spellings.

function attempt(label: string, fn: () => any): void {
  try {
    console.log(label + "=" + String(fn()));
  } catch (e) {
    console.log(label + "!" + (e as any).name);
  }
}

// --- DataView: setBigInt64/setBigUint64 over the same eight bytes ---
const buf = new ArrayBuffer(16);
const view = new DataView(buf);
const bytes = new Uint8Array(buf);

function hex(offset: number): string {
  let out = "";
  for (let i = offset; i < offset + 8; i++) out += bytes[i].toString(16).padStart(2, "0");
  return out;
}

function write(label: string, v: bigint): void {
  view.setBigInt64(0, v, false);
  const be = hex(0);
  view.setBigInt64(0, v, true);
  const le = hex(0);
  view.setBigInt64(0, v, false);
  console.log(
    label + " | be:" + be + " | le:" + le +
      " | signed:" + String(view.getBigInt64(0, false)) +
      " | unsigned:" + String(view.getBigUint64(0, false))
  );
}
write("0n", 0n);
write("1n", 1n);
write("-1n", -1n);
write("255n", 255n);
write("-255n", -255n);
write("2^32", 2n ** 32n);
write("2^63-1", 2n ** 63n - 1n);
write("-2^63", -(2n ** 63n));
write("max_safe", BigInt(Number.MAX_SAFE_INTEGER));
write("0x0123456789abcdef", 0x0123456789abcdefn);

// --- the two endiannesses are byte reversals of each other ---
view.setBigUint64(0, 0x0123456789abcdefn, false);
const beBytes: number[] = [];
for (let i = 0; i < 8; i++) beBytes.push(bytes[i]);
view.setBigUint64(0, 0x0123456789abcdefn, true);
const leBytes: number[] = [];
for (let i = 0; i < 8; i++) leBytes.push(bytes[i]);
console.log("be_bytes=" + beBytes.join(","));
console.log("le_bytes=" + leBytes.join(","));
console.log("is_reversal=" + (beBytes.join(",") === leBytes.slice().reverse().join(",")));
view.setBigUint64(0, 1n, false);
console.log("default_flag_is_big_endian=" + (hex(0) === "0000000000000001"));

// --- the unsigned reading of a negative value is its two's complement ---
view.setBigInt64(0, -1n, false);
console.log("neg_one_unsigned=" + String(view.getBigUint64(0, false)) + " is_2p64_minus_1=" + (view.getBigUint64(0, false) === 2n ** 64n - 1n));
view.setBigInt64(0, -(2n ** 63n), false);
console.log("min_i64_unsigned=" + String(view.getBigUint64(0, false)) + " asUintN_agrees=" + (view.getBigUint64(0, false) === BigInt.asUintN(64, -(2n ** 63n))));
view.setBigUint64(0, 2n ** 64n - 1n, false);
console.log("max_u64_signed=" + String(view.getBigInt64(0, false)) + " asIntN_agrees=" + (view.getBigInt64(0, false) === BigInt.asIntN(64, 2n ** 64n - 1n)));

// --- values outside the 64-bit range wrap rather than throw ---
view.setBigInt64(0, 2n ** 64n + 5n, false);
console.log("wrap_above=" + String(view.getBigInt64(0, false)));
view.setBigUint64(0, -5n, false);
console.log("wrap_negative_unsigned=" + String(view.getBigUint64(0, false)));
view.setBigInt64(0, 2n ** 200n, false);
console.log("wrap_huge=" + String(view.getBigInt64(0, false)));

// --- these entry points refuse a Number, string, or null without coercing ---
attempt("set_number", () => { view.setBigInt64(0, 1 as any, false); return "written"; });
attempt("set_numeric_string", () => { view.setBigInt64(0, "1" as any, false); return "written"; });
attempt("set_null", () => { view.setBigUint64(0, null as any, false); return "written"; });
attempt("set_boolean", () => { view.setBigUint64(0, true as any, false); return "written"; });
attempt("float64_rejects_bigint", () => { view.setFloat64(0, 1n as any, false); return "written"; });
attempt("uint32_rejects_bigint", () => { view.setUint32(0, 1n as any, false); return "written"; });

// --- offsets are checked against the buffer length ---
attempt("offset_8_ok", () => { view.setBigInt64(8, 42n, false); return String(view.getBigInt64(8, false)); });
attempt("offset_9_overruns", () => String(view.getBigInt64(9, false)));
attempt("offset_negative", () => String(view.getBigInt64(-1, false)));

// --- typed arrays over the same buffer read the same bytes ---
const i64 = new BigInt64Array(buf);
const u64 = new BigUint64Array(buf);
view.setBigInt64(0, -2n, true);
view.setBigInt64(8, 7n, true);
console.log("i64_view=" + Array.from(i64).map(String).join(","));
console.log("u64_view=" + Array.from(u64).map(String).join(","));
console.log("byte_length=" + i64.byteLength + " length=" + i64.length + " BYTES_PER_ELEMENT=" + BigInt64Array.BYTES_PER_ELEMENT);
attempt("typed_array_number_assign", () => { (i64 as any)[0] = 5; return String(i64[0]); });
attempt("typed_array_from_numbers", () => Array.from(BigInt64Array.from([1, 2] as any)).join(","));
attempt("typed_array_from_bigints", () => Array.from(BigInt64Array.from([1n, 2n])).map(String).join(","));
attempt("typed_array_sort_default", () => Array.from(BigInt64Array.from([3n, -1n, 2n]).sort()).map(String).join(","));

// --- BigUint64Array wraps negatives and overflows to the element width ---
const unsigned = new BigUint64Array(3);
unsigned[0] = -1n;
unsigned[1] = 2n ** 64n;
unsigned[2] = 5n;
console.log("u64_store=" + Array.from(unsigned).map(String).join(","));
const signed = new BigInt64Array(3);
signed[0] = -1n;
signed[1] = 2n ** 63n;
signed[2] = -(2n ** 63n) - 1n;
console.log("i64_store=" + Array.from(signed).map(String).join(","));

// --- JSON.stringify refuses a BigInt wherever it appears ---
attempt("json_bare", () => JSON.stringify(1n));
attempt("json_zero", () => JSON.stringify(0n));
attempt("json_in_object", () => JSON.stringify({ a: 1n }));
attempt("json_in_array", () => JSON.stringify([1n]));
attempt("json_nested", () => JSON.stringify({ a: { b: [1n] } }));
attempt("json_wrapper", () => JSON.stringify(Object(1n)));
attempt("json_as_key", () => JSON.stringify({ [1n as any]: 2 }));

// --- a replacer that converts first is the ordinary workaround ---
attempt("replacer_string", () =>
  JSON.stringify({ a: 1n, b: 2 }, function (_k: string, v: any) {
    return typeof v === "bigint" ? v.toString() : v;
  })
);
attempt("replacer_number", () =>
  JSON.stringify({ a: 5n }, function (_k: string, v: any) {
    return typeof v === "bigint" ? Number(v) : v;
  })
);
attempt("replacer_sees_bigint", () => {
  const types: string[] = [];
  try {
    JSON.stringify({ a: 1n }, function (_k: string, v: any) {
      types.push(typeof v);
      return typeof v === "bigint" ? "converted" : v;
    });
  } catch (e) {
    types.push("threw");
  }
  return types.join(",");
});

// --- an object's own toJSON is consulted before the BigInt type check ---
attempt("own_toJSON", () => JSON.stringify({ a: { value: 1n, toJSON: function () { return "one"; } } }));

// --- the toJSON escape hatch on BigInt.prototype ---
(BigInt.prototype as any).toJSON = function () {
  return this.toString();
};
attempt("with_toJSON_bare", () => JSON.stringify(1n));
attempt("with_toJSON_object", () => JSON.stringify({ a: 1n, b: [2n] }));
attempt("with_toJSON_wrapper", () => JSON.stringify(Object(3n)));
delete (BigInt.prototype as any).toJSON;
attempt("after_delete", () => JSON.stringify(1n));
console.log("toJSON_gone=" + ("toJSON" in BigInt.prototype));

// --- JSON.parse never yields a BigInt; long integers lose precision ---
const parsed: any = JSON.parse("123456789012345678901234567890");
console.log("parsed_typeof=" + typeof parsed + " parsed_value=" + String(parsed));
const parsed2: any = JSON.parse('{"n":9007199254740993}');
console.log("parsed_lost=" + String(parsed2.n) + " collides=" + (parsed2.n === 9007199254740992));
console.log("bigint_from_json_text=" + BigInt(String(parsed2.n)).toString());

// --- default sort compares decimal spellings; numeric comparator fixes it ---
const nums: any[] = [10n, 9n, 1n, -1n, 2n, 100n, 0n];
console.log("default_sort=" + nums.slice().sort().join(","));
console.log("numeric_sort=" + nums.slice().sort((a: any, b: any) => (a < b ? -1 : a > b ? 1 : 0)).join(","));
console.log("default_sort_literal=" + [10n, 9n, 100n, 2n].sort().join(","));
console.log("mixed_default=" + ([10n, 9, 1n, 2] as any[]).slice().sort().join(","));
console.log("to_sorted=" + (nums as any).toSorted().join(","));
attempt("comparator_returning_bigint", () => nums.slice().sort((a: any, b: any) => (a - b) as any).join(","));

// --- keys of an object built from BigInts are their decimal strings ---
console.log("object_keys=" + Object.keys({ [1n as any]: 0, [2n as any]: 0 }).join(","));
console.log("huge_bigint_key=" + Object.keys({ [2n ** 70n as any]: 1 })[0]);
