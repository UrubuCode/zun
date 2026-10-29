// console.log printf-style format specifiers: %s %d %i %f %j %% substitution,
// leftover-argument joining, and unmatched specifiers left as written.
// Only behavior that Bun and Node agree on is pinned here. Object/array
// pretty-printing and %o/%O are deliberately excluded (they differ by runtime).
// Every line is produced by a single console.log call so the output is one line
// per case; the trailing "|" marks where the substituted text ends.

// %s stringifies a primitive; objects use their toString.
console.log("s_string=%s|", "abc");
console.log("s_number=%s|", 42);
console.log("s_float=%s|", 4.5);
console.log("s_null=%s|", null);
console.log("s_undefined=%s|", undefined);
console.log("s_true=%s|", true);
console.log("s_empty=%s|", "");
console.log("s_via_toString=%s|", { toString() { return "TS"; } });
console.log("s_substituted_text_has_percent=%s|", "50%");
console.log("s_argument_contains_specifier=%s|", "%d");

// %d and %i coerce with ToNumber; %i then truncates toward zero.
console.log("d_int=%d|", 42);
console.log("d_numeric_string=%d|", "12");
console.log("d_padded_string=%d|", " 12 ");
console.log("d_empty_string=%d|", "");
console.log("d_hex_string=%d|", "0x10");
console.log("d_garbage=%d|", "xy");
console.log("d_null=%d|", null);
console.log("d_true=%d|", true);
console.log("d_undefined=%d|", undefined);
console.log("d_nan=%d|", NaN);
console.log("d_valueOf=%d|", { valueOf() { return 7; } });
console.log("d_negative=%d|", -3);

console.log("i_truncates=%i|", 4.7);
console.log("i_truncates_negative=%i|", -4.7);
console.log("i_string=%i|", "-12");
console.log("i_hex=%i|", "0x10");
console.log("i_garbage=%i|", "xy");

// %f keeps the fraction.
console.log("f_float=%f|", 4.75);
console.log("f_string=%f|", "3.5");
console.log("f_leading_dot=%f|", ".5");
console.log("f_exponent=%f|", "1e3");
console.log("f_repeating=%f|", 1 / 3);
console.log("f_garbage=%f|", "xy");

// %j is JSON.
console.log("j_object=%j|", { a: 1 });
console.log("j_array=%j|", [1, "a", null]);
console.log("j_string=%j|", "str");
console.log("j_number=%j|", 5);
console.log("j_nested=%j|", { a: { b: [true, false] } });

// %% is a literal percent when there IS an argument to substitute.
console.log("pct_no_args=%%|");
console.log("pct_with_arg=%%|", "a");
console.log("pct_before_letter=%%d|", 1);
console.log("trailing_pct=100%|", "a");
console.log("pct_between=a%%b|", "x");

// Several specifiers consume arguments left to right.
console.log("two=%s and %s|", "1", "2");
console.log("mixed=%s/%d/%f|", "x", "9", "1.5");
console.log("order=%s-%s-%s|", "a", "b", "c");

// Arguments left over are appended, space-separated.
console.log("leftover=%s|", "a", "extra", 1);
console.log("no_specifier=", "a", 1);
console.log("leftover_only=", "x", "y");

// A specifier with no argument is left exactly as written.
console.log("missing_arg=%s|");
console.log("partly_missing=%s/%s|", "a");

// An unknown specifier is not a specifier.
console.log("unknown=%z|", "a");

// No arguments at all, and an empty string, each print one empty line.
console.log();
console.log("");

// A newline inside the format string still splits the output.
console.log("first\nsecond=%s|", "tail");

// A specifier in a LATER argument is plain text, not a format.
console.log("literal=%s|", "%d", 5);

// console.log always returns undefined.
console.log("returns=" + String(console.log()));
console.log("done");
