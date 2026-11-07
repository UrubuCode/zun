mod common;
use common::*;

use zun_interp::value::Value;

#[test]
fn number_primitive_methods_use_static_class_binding() {
    let module = zun_compiler::compile(
        "import { toFixed } from \"zun:core\";\n\
         class Number { extern toFixed(digits: int): string; }\n\
         function main(): string { let value: number = 12.5; return value.toFixed(1); }",
    )
    .expect("compile Number methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(
        !jit.is_compiled(entry),
        "native Number methods must remain Tier 0"
    );
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "12.5"));
}

#[test]
fn number_sign_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { sign(): number { if (this < 0.0) { return -1.0; } if (this > 0.0) { return 1.0; } return 0.0; } }\n\
         function main(): number { let value: number = -42.0; return value.sign(); }",
    )
    .expect("compile Number sign");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == -1.0));
}

#[test]
fn number_finiteness_methods_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isNaN(): boolean { return this != this; } isFinite(): boolean { return this == this; } }\n\
         function main(): boolean { let value: number = 42.0; if (!value.isFinite()) { return false; } return !value.isNaN(); }",
    )
    .expect("compile Number finiteness methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_is_integer_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isInteger(): boolean { return (this as int) as number == this; } }\n\
         function main(): boolean { let value: number = 42.0; return value.isInteger(); }",
    )
    .expect("compile Number isInteger");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_parity_methods_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isInteger(): boolean { return (this as int) as number == this; } isEven(): boolean { if (!this.isInteger()) { return false; } return (this as int) % 2 == 0; } isOdd(): boolean { if (!this.isInteger()) { return false; } return (this as int) % 2 != 0; } }\n\
         function main(): boolean { let value: number = 42.0; if (!value.isEven()) { return false; } return !value.isOdd(); }",
    )
    .expect("compile Number parity methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_to_int_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { toInt(): int { return this as int; } }\n\
         function main(): int { let value: number = 12.75; return value.toInt(); }",
    )
    .expect("compile Number toInt");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 12));
}

#[test]
fn number_to_boolean_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { toBoolean(): boolean { return this != 0.0; } }\n\
         function main(): boolean { let value: number = 1.0; return value.toBoolean(); }",
    )
    .expect("compile Number toBoolean");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_to_string_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { toString } from \"zun:core\";\n\
         class Number { extern toString(): string; }\n\
         function main(): string { let value: number = 12.5; return value.toString(); }",
    )
    .expect("compile Number toString");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "12.5"));
}

#[test]
fn number_source_methods_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { abs(): number { if (this < 0.0) { return -this; } return this; } isZero(): boolean { return this == 0.0; } isNegative(): boolean { return this < 0.0; } }\n\
         function main(): number { let value: number = -7.5; if (value.isNegative()) { return value.abs(); } return 0.0; }",
    )
    .expect("compile Number source methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(
        !jit.is_compiled(entry),
        "Number method calls must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(7.5, zun_il::NumKind::F64)
    ));
}

#[test]
fn number_clamp_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { clamp(low: number, high: number): number { if (this < low) { return low; } if (this > high) { return high; } return this; } }\n\
         function main(): number { let value: number = 12.0; return value.clamp(0.0, 10.0); }",
    )
    .expect("compile Number clamp");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(
        !jit.is_compiled(entry),
        "Number method calls must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(10.0, zun_il::NumKind::F64)
    ));
}

#[test]
fn number_min_and_max_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { min(other: number): number { if (this < other) { return this; } return other; } max(other: number): number { if (this > other) { return this; } return other; } }\n\
         function main(): number { let value: number = 7.0; return value.min(10.0) + value.max(3.0); }",
    )
    .expect("compile Number min max");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(
        !jit.is_compiled(entry),
        "Number method calls must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(14.0, zun_il::NumKind::F64)
    ));
}

#[test]
fn string_source_methods_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class String { concat(other: string): string { return this + other; } withSeparator(separator: string, other: string): string { return this + separator + other; } }\n\
         function main(): string { let value: string = \"zun\"; return value.withSeparator(\":\", \"vm\"); }",
    )
    .expect("compile String source methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(
        !jit.is_compiled(entry),
        "String method calls must remain Tier 0"
    );
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun:vm"));
}

#[test]
fn string_is_empty_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length } from \"zun:core\";\n\
         class String { extern length(): u64; isEmpty(): boolean { return this.length() == 0; } }\n\
         function main(): boolean { let value: string = \"\"; return value.isEmpty(); }",
    )
    .expect("compile String isEmpty");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_to_string_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class String { toString(): string { return this; } }\n\
         function main(): string { let value: string = \"zun\"; return value.toString(); }",
    )
    .expect("compile String toString");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun"));
}

#[test]
fn array_source_methods_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array {\n\
             contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; }\n\
         }\n\
         function main(): boolean { let values = new Array(1); values[0] = 41; return values.contains(values[0]); }",
    )
    .expect("compile Array source method");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_higher_order_methods_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { map(callback: Function): Array { let result = new Array(this.length); let index: u64 = 0; while (index < this.length) { result[index] = callback(this[index]); index = index + 1; } return result; } filter(callback: Predicate): Array { let result = new Array(0); let index: u64 = 0; while (index < this.length) { let value: any = this[index]; if (callback(value)) { result.push(value); } index = index + 1; } return result; } reduce(callback: Reducer, initial: any): any { let accumulator: any = initial; let index: u64 = 0; while (index < this.length) { accumulator = callback(accumulator, this[index]); index = index + 1; } return accumulator; } }\n\
         function add_one(value: any): any { return (value as number) + 1.0; }\n\
         function positive(value: any): boolean { return (value as number) > 0.0; }\n\
         function add(accumulator: any, value: any): any { return (accumulator as number) + (value as number); }\n\
         function main(): any { let values = new Array(2); values[0] = -1.0; values[1] = 41.0; let mapped = values.map(add_one); let filtered = mapped.filter(positive); return filtered.reduce(add, 0.0); }",
    )
    .expect("compile Array higher-order methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with callback fallback");
    assert!(
        !jit.is_compiled(entry),
        "indirect callback methods must remain Tier 0"
    );
    assert!(
        matches!(interp_entry(&module), Value::Float(value, zun_il::NumKind::F64) if value == 42.0)
    );
}

#[test]
fn non_capturing_closure_keeps_interpreter_fallback_boundary() {
    let module = zun_compiler::compile(
        "function main(): number { let double = (value: number) => value * 2.0; return double(21.0) as number; }",
    )
    .expect("compile non-capturing closure");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with closure fallback");
    assert!(
        !jit.is_compiled(entry),
        "closure calls require Tier 0 environment ABI"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(42.0, zun_il::NumKind::F64)
    ));
}

#[test]
fn array_count_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { count(value: any): u64 { let total: u64 = 0; let index: u64 = 0; while (index < this.length) { if (this[index] == value) { total = total + 1; } index = index + 1; } return total; } }\n\
         function main(): u64 { let values = new Array(4); values[0] = 7; values[1] = 3; values[2] = 7; values[3] = 7; let needle: any = values[0]; return values.count(needle); }",
    )
    .expect("compile Array count");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 3));
}

#[test]
fn array_clear_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { clear(): void { while (this.length > 0) { this.pop(); } } }\n\
         function main(): u64 { let values = new Array(2); values[0] = 7; values[1] = 3; values.clear(); return values.length; }",
    )
    .expect("compile Array clear");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 0));
}

#[test]
fn array_average_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { sum(): number { let total: number = 0.0; let index: u64 = 0; while (index < this.length) { total = total + (this[index] as number); index = index + 1; } return total; } average(): number { if (this.length == 0) { return 0.0; } return this.sum() / (this.length as number); } }\n\
         function main(): number { let values = new Array(2); values[0] = 2.0; values[1] = 6.0; return values.average(); }",
    )
    .expect("compile Array average");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 4.0));
}

#[test]
fn array_min_and_max_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { min(): number { if (this.length == 0) { return 0.0; } let result: number = this[0] as number; let index: u64 = 1; while (index < this.length) { let value: number = this[index] as number; if (value < result) { result = value; } index = index + 1; } return result; } max(): number { if (this.length == 0) { return 0.0; } let result: number = this[0] as number; let index: u64 = 1; while (index < this.length) { let value: number = this[index] as number; if (value > result) { result = value; } index = index + 1; } return result; } }\n\
         function main(): number { let values = new Array(3); values[0] = 9.0; values[1] = 2.0; values[2] = 7.0; return values.min() + values.max(); }",
    )
    .expect("compile Array min and max");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 11.0));
}

#[test]
fn array_sort_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { sort(): Array { let index: u64 = 0; while (index < this.length) { let smallest: u64 = index; let scan: u64 = index + 1; while (scan < this.length) { let candidate: number = this[scan] as number; let currentSmallest: number = this[smallest] as number; if (candidate < currentSmallest) { smallest = scan; } scan = scan + 1; } let temporary: any = this[index]; this[index] = this[smallest]; this[smallest] = temporary; index = index + 1; } return this; } }\n\
         function main(): number { let values = new Array(3); values[0] = 9.0; values[1] = 2.0; values[2] = 7.0; values.sort(); return values[0] as number; }",
    )
    .expect("compile Array sort");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 2.0));
}

#[test]
fn array_find_index_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        // `findIndex` now takes an ECMAScript predicate `(value, index, array)`
        // (previously a value, a Zun divergence); it stays Tier 0 because the
        // callback re-enters the interpreter.
        "function is_eight(value: any, index: any, array: any): boolean { return (value as number) == 8.0; }\n\
         function main(): int { let values = new Array(3); values[0] = 4.0; values[1] = 8.0; values[2] = 15.0; return values.findIndex(is_eight); }",
    )
    .expect("compile Array findIndex");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 1));
}

#[test]
fn array_last_index_of_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { lastIndexOf(value: any): int { let index: int = this.length as int; while (index > 0) { index = index - 1; if (this[index as u64] == value) { return index; } } return -1; } }\n\
         function main(): int { let values = new Array(3); values[0] = 4; values[1] = 8; values[2] = 8; let needle: any = values[1]; return values.lastIndexOf(needle); }",
    )
    .expect("compile Array lastIndexOf");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 2));
}

#[test]
fn array_swap_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { swap(first: u64, second: u64): Array { let temporary: any = this[first]; this[first] = this[second]; this[second] = temporary; return this; } }\n\
         function main(): number { let values = new Array(2); values[0] = 4.0; values[1] = 8.0; values.swap(0, 1); return values[0] as number; }",
    )
    .expect("compile Array swap");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 8.0));
}

#[test]
fn array_rotate_left_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { rotateLeft(shift: u64): Array { if (this.length == 0) { return this; } let amount: u64 = shift % this.length; let rotation: u64 = 0; while (rotation < amount) { let first: any = this[0]; let index: u64 = 1; while (index < this.length) { this[index - 1] = this[index]; index = index + 1; } this[this.length - 1] = first; rotation = rotation + 1; } return this; } }\n\
         function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values.rotateLeft(1); return values[0] as number; }",
    )
    .expect("compile Array rotateLeft");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 2.0));
}

#[test]
fn array_rotate_right_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { rotateRight(shift: u64): Array { if (this.length == 0) { return this; } let amount: u64 = shift % this.length; let rotation: u64 = 0; while (rotation < amount) { let last: any = this[this.length - 1]; let index: u64 = this.length - 1; while (index > 0) { this[index] = this[index - 1]; index = index - 1; } this[0] = last; rotation = rotation + 1; } return this; } }\n\
         function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values.rotateRight(1); return values[0] as number; }",
    )
    .expect("compile Array rotateRight");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 3.0));
}

#[test]
fn array_take_and_drop_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { slice(start: u64, end: u64): Array { let count: u64 = end - start; let result = new Array(count); let index: u64 = 0; while (index < count) { result[index] = this[start + index]; index = index + 1; } return result; } take(count: u64): Array { let limit: u64 = count; if (limit > this.length) { limit = this.length; } return this.slice(0, limit); } drop(count: u64): Array { let start: u64 = count; if (start > this.length) { start = this.length; } return this.slice(start, this.length); } } function main(): number { let values = new Array(3); values[0] = 10.0; values[1] = 20.0; values[2] = 30.0; let prefix = values.take(2); let suffix = values.drop(1); return prefix[1] as number + suffix[0] as number; }",
    )
    .expect("compile Array take/drop");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    let result = interp_entry(&module);
    assert!(matches!(result, Value::Float(value, _) if value == 40.0));
}

#[test]
fn array_remove_at_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { removeAt(index: u64): any { let removed: any = this[index]; let cursor: u64 = index; while (cursor + 1 < this.length) { this[cursor] = this[cursor + 1]; cursor = cursor + 1; } this.pop(); return removed; } } function main(): number { let values = new Array(3); values[0] = 10.0; values[1] = 20.0; values[2] = 30.0; let removed: any = values.removeAt(1); return (removed as number) + (values[1] as number); }",
    )
    .expect("compile Array removeAt");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 50.0));
}

#[test]
fn array_insert_at_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { insertAt(index: u64, value: any): Array { let cursor: u64 = this.length; this.push(value); while (cursor > index) { this[cursor] = this[cursor - 1]; cursor = cursor - 1; } this[index] = value; return this; } } function main(): number { let values = new Array(2); values[0] = 10.0; values[1] = 30.0; values.insertAt(1, values[0]); return (values[1] as number) + (values[2] as number); }",
    )
    .expect("compile Array insertAt");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 40.0));
}

#[test]
fn array_remove_first_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { removeAt(index: u64): any { let removed: any = this[index]; let cursor: u64 = index; while (cursor + 1 < this.length) { this[cursor] = this[cursor + 1]; cursor = cursor + 1; } this.pop(); return removed; } removeFirst(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { this.removeAt(index); return true; } index = index + 1; } return false; } } function main(): number { let values = new Array(3); values[0] = 10.0; values[1] = 20.0; values[2] = 30.0; let needle: any = values[1]; if (!values.removeFirst(needle)) { return -1.0; } return values[1] as number; }",
    )
    .expect("compile Array removeFirst");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 30.0));
}

#[test]
fn array_remove_all_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { removeAt(index: u64): any { let removed: any = this[index]; let cursor: u64 = index; while (cursor + 1 < this.length) { this[cursor] = this[cursor + 1]; cursor = cursor + 1; } this.pop(); return removed; } removeAll(value: any): u64 { let removed: u64 = 0; let index: u64 = 0; while (index < this.length) { if (this[index] == value) { this.removeAt(index); removed = removed + 1; } else { index = index + 1; } } return removed; } } function main(): number { let values = new Array(4); values[0] = 7.0; values[1] = 3.0; values[2] = 7.0; values[3] = 5.0; let needle: any = values[0]; return values.removeAll(needle) as number; }",
    )
    .expect("compile Array removeAll");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 2.0));
}

#[test]
fn array_equals_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { equals(other: Array): boolean { if (this.length != other.length) { return false; } let index: u64 = 0; while (index < this.length) { if (this[index] != other[index]) { return false; } index = index + 1; } return true; } } function main(): boolean { let left = new Array(2); left[0] = 4.0; left[1] = 8.0; let right = new Array(2); right[0] = 4.0; right[1] = 8.0; return left.equals(right); }",
    )
    .expect("compile Array equals");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_sorted_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isSorted(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current < previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 2.0; return values.isSorted(); }",
    )
    .expect("compile Array isSorted");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_strictly_increasing_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isStrictlyIncreasing(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current <= previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; return values.isStrictlyIncreasing(); }",
    )
    .expect("compile Array isStrictlyIncreasing");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_strictly_decreasing_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isStrictlyDecreasing(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current >= previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 3.0; values[1] = 2.0; values[2] = 1.0; return values.isStrictlyDecreasing(); }",
    )
    .expect("compile Array isStrictlyDecreasing");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_descending_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isDescending(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current > previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 3.0; values[1] = 2.0; values[2] = 2.0; return values.isDescending(); }",
    )
    .expect("compile Array isDescending");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_unique_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isUnique(): boolean { let outer: u64 = 0; while (outer < this.length) { let inner: u64 = outer + 1; while (inner < this.length) { if (this[outer] == this[inner]) { return false; } inner = inner + 1; } outer = outer + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; return values.isUnique(); }",
    )
    .expect("compile Array isUnique");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_transpose_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { transpose(): Array { if (this.length == 0) { return new Array(0); } let first: Array = this[0] as Array; let rows: u64 = this.length; let columns: u64 = first.length; let result = new Array(columns); let column: u64 = 0; while (column < columns) { let rowResult = new Array(rows); let row: u64 = 0; while (row < rows) { let source: Array = this[row] as Array; rowResult[row] = source[column]; row = row + 1; } result[column] = rowResult; column = column + 1; } return result; } } function main(): number { let matrix = new Array(2); let first = new Array(2); first[0] = 1.0; first[1] = 2.0; let second = new Array(2); second[0] = 3.0; second[1] = 4.0; matrix[0] = first; matrix[1] = second; let result = matrix.transpose(); let column = result[1] as Array; return column[0] as number; }",
    )
    .expect("compile Array transpose");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 2.0));
}

#[test]
fn array_chunk_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { chunk(size: u64): Array { if (size == 0) { return new Array(0); } let groups: u64 = this.length / size; if (this.length % size != 0) { groups = groups + 1; } let result = new Array(groups); let group: u64 = 0; let start: u64 = 0; while (group < groups) { let remaining: u64 = this.length - start; let count: u64 = size; if (remaining < count) { count = remaining; } let part = new Array(count); let index: u64 = 0; while (index < count) { part[index] = this[start + index]; index = index + 1; } result[group] = part; start = start + count; group = group + 1; } return result; } } function main(): number { let values = new Array(5); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values[3] = 4.0; values[4] = 5.0; let groups = values.chunk(2); let last = groups[2] as Array; return last[0] as number; }",
    )
    .expect("compile Array chunk");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 5.0));
}

#[test]
fn array_windowed_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { windowed(size: u64): Array { if (size == 0) { return new Array(0); } if (size > this.length) { return new Array(0); } let count: u64 = this.length - size + 1; let result = new Array(count); let start: u64 = 0; while (start < count) { let window = new Array(size); let index: u64 = 0; while (index < size) { window[index] = this[start + index]; index = index + 1; } result[start] = window; start = start + 1; } return result; } } function main(): number { let values = new Array(4); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values[3] = 4.0; let windows = values.windowed(3); let first = windows[0] as Array; return first[0] as number; }",
    )
    .expect("compile Array windowed");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 1.0));
}

#[test]
fn array_product_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { product(): number { let total: number = 1.0; let index: u64 = 0; while (index < this.length) { total = total * (this[index] as number); index = index + 1; } return total; } }\n\
         function main(): number { let values = new Array(3); values[0] = 2.0; values[1] = 3.0; values[2] = 4.0; return values.product(); }",
    )
    .expect("compile Array product");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 24.0));
}
