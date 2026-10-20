//! Tier 0 vs Tier 1 equivalence for the typed JIT.
//!
//! Each test compiles a program whose `main` is a nullary numeric entry, runs
//! it through both the interpreter and the Cranelift JIT, and requires matching
//! results — now across integer widths, signedness, and f32, not just f64.

use zun_codegen::RetVal;
use zun_interp::value::Value;
use zun_interp::CaptureOutput;

fn compile(src: &str) -> zun_module::Module {
    let mut diags = Vec::new();
    let ast = zun_syntax::parse(src, &mut diags);
    assert!(diags.is_empty(), "parse: {diags:?}");
    let hir = zun_sema::check(src, &ast, &mut diags);
    assert!(diags.is_empty(), "sema: {diags:?}");
    zun_ilgen::lower(&hir)
}

/// Run the entry point through the interpreter, returning its value.
fn interp_entry(module: &zun_module::Module) -> Value {
    let mut out = CaptureOutput::default();
    zun_interp::run(module, &mut out).unwrap()
}

/// Assert the JIT and interpreter agree on the entry point's result.
fn assert_equivalent(src: &str) {
    let module = compile(src);
    let entry = module.entry_point.expect("entry point").rid();

    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry), "entry should be JIT-eligible:\n{src}");

    let jit_result = jit.call_entry(entry).expect("jit call");
    let interp_result = interp_entry(&module);

    let matches = match (&jit_result, &interp_result) {
        (RetVal::Float(a), Value::Float(b, _)) => a == b,
        (RetVal::Int(a), Value::Int(b, _)) => a == b,
        (RetVal::Bool(a), Value::Bool(b)) => a == b,
        _ => false,
    };
    assert!(
        matches,
        "JIT {jit_result:?} vs interpreter {interp_result:?} for:\n{src}"
    );
}

#[test]
fn f64_arithmetic_and_recursion() {
    assert_equivalent(
        "function fib(n: number): number {\n\
            if (n < 2) { return n; }\n\
            return fib(n - 1) + fib(n - 2);\n\
         }\n\
         function main(): number { return fib(15); }",
    );
}

#[test]
fn int_arithmetic() {
    assert_equivalent(
        "function main(): int { let a: int = 10; let b: int = 3; return a * b + a / b; }",
    );
}

#[test]
fn int_recursion() {
    assert_equivalent(
        "function sumTo(n: int): int { if (n == 0) { return 0; } return n + sumTo(n - 1); }\n\
         function main(): int { return sumTo(100); }",
    );
}

#[test]
fn extern_method_uses_interpreter_fallback_until_native_jit_lowering_exists() {
    let module = zun_compiler::compile(
        "import { receiverValue } from \"zun:math\";\n\
         class Service { extern receiverValue(value: number): number; }\n\
         function main(): number { let service = new Service(); return service.receiverValue(7.5); }",
    )
    .expect("compile extern method");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with fallback methods");
    assert!(!jit.is_compiled(entry), "native builtin call must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 7.5));
}

#[test]
fn string_primitive_methods_use_static_class_binding() {
    let module = zun_compiler::compile(
        "import { length, contains } from \"zun:string\";\n\
         class String { extern length(): u64; extern contains(needle: string): boolean; }\n\
         function main(): boolean { let value: string = \"zun\"; if (value.length() == 3) { return value.contains(\"u\"); } return false; }",
    )
    .expect("compile String methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "native String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_index_of_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { indexOf } from \"zun:string\";\n\
         class String { extern indexOf(needle: string): int; }\n\
         function main(): int { let value: string = \"😀zun\"; return value.indexOf(\"zun\"); }",
    )
    .expect("compile String indexOf");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(2, zun_il::NumKind::I32)));
}

#[test]
fn string_last_index_of_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { lastIndexOf } from \"zun:string\";\n\
         class String { extern lastIndexOf(needle: string): int; }\n\
         function main(): int { let value: string = \"zunzun\"; return value.lastIndexOf(\"zun\"); }",
    )
    .expect("compile String lastIndexOf");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(3, zun_il::NumKind::I32)));
}

#[test]
fn string_replace_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { replace } from \"zun:string\";\n\
         class String { extern replace(needle: string, replacement: string): string; }\n\
         function main(): string { let value: string = \"zun vm\"; return value.replace(\"vm\", \"runtime\"); }",
    )
    .expect("compile String replace");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun runtime"));
}

#[test]
fn string_substring_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { substring } from \"zun:string\";\n\
         class String { extern substring(start: u64, end: u64): string; }\n\
         function main(): string { let value: string = \"😀zun\"; return value.substring(2, 5); }",
    )
    .expect("compile String substring");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun"));
}

#[test]
fn string_pad_start_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { padStart } from \"zun:string\";\n\
         class String { extern padStart(targetLength: u64, fill: string): string; }\n\
         function main(): string { let value: string = \"zun\"; return value.padStart(5, \"0\"); }",
    )
    .expect("compile String padStart");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "00zun"));
}

#[test]
fn string_pad_end_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { padEnd } from \"zun:string\";\n\
         class String { extern padEnd(targetLength: u64, fill: string): string; }\n\
         function main(): string { let value: string = \"zun\"; return value.padEnd(5, \"0\"); }",
    )
    .expect("compile String padEnd");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun00"));
}

#[test]
fn number_primitive_methods_use_static_class_binding() {
    let module = zun_compiler::compile(
        "import { toFixed } from \"zun:number\";\n\
         class Number { extern toFixed(digits: int): string; }\n\
         function main(): string { let value: number = 12.5; return value.toFixed(1); }",
    )
    .expect("compile Number methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "native Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "12.5"));
}

#[test]
fn number_to_string_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { toString } from \"zun:number\";\n\
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
    assert!(!jit.is_compiled(entry), "Number method calls must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(7.5, zun_il::NumKind::F64)));
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
    assert!(!jit.is_compiled(entry), "Number method calls must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(10.0, zun_il::NumKind::F64)));
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
    assert!(!jit.is_compiled(entry), "Number method calls must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(14.0, zun_il::NumKind::F64)));
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
    assert!(!jit.is_compiled(entry), "String method calls must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun:vm"));
}

#[test]
fn string_is_empty_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length } from \"zun:string\";\n\
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
    assert!(!jit.is_compiled(entry), "Array instructions must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_search_methods_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array {\n\
             indexOf(value: any): int { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return index as int; } index = index + 1; } return -1; }\n\
             isEmpty(): boolean { return this.length == 0; }\n\
         }\n\
         function main(): int { let values = new Array(1); values[0] = 41; if (values.isEmpty()) { return -2; } return values.indexOf(values[0]); }",
    )
    .expect("compile Array search methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array instructions must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(0, zun_il::NumKind::I32)));
}

#[test]
fn array_access_methods_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { at(index: u64): any { return this[index]; } first(): any { return this[0]; } last(): any { return this[this.length - 1]; } }\n\
         function main(): number { let values = new Array(2); values[0] = 7.0; values[1] = 9.0; return values.at(1) as number; }",
    )
    .expect("compile Array access methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array instructions must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(9.0, zun_il::NumKind::F64)));
}

#[test]
fn array_reverse_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { reverse(): Array { let left: u64 = 0; if (this.length == 0) { return this; } let right: u64 = this.length - 1; while (left < right) { let temporary: any = this[left]; this[left] = this[right]; this[right] = temporary; left = left + 1; right = right - 1; } return this; } first(): any { return this[0]; } }\n\
         function main(): number { let values = new Array(2); values[0] = 7.0; values[1] = 9.0; values.reverse(); return values.first() as number; }",
    )
    .expect("compile Array reverse");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array instructions must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(9.0, zun_il::NumKind::F64)));
}

#[test]
fn array_fill_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { fill(value: any): Array { let index: u64 = 0; while (index < this.length) { this[index] = value; index = index + 1; } return this; } first(): any { return this[0]; } }\n\
         function main(): number { let values = new Array(2); values[0] = 9.0; values.fill(values[0]); return values.first() as number; }",
    )
    .expect("compile Array fill");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array instructions must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(9.0, zun_il::NumKind::F64)));
}

#[test]
fn array_slice_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { slice(start: u64, end: u64): Array { let count: u64 = end - start; let result = new Array(count); let index: u64 = 0; while (index < count) { result[index] = this[start + index]; index = index + 1; } return result; } first(): any { return this[0]; } }\n\
         function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 7.0; values[2] = 9.0; let selected = values.slice(1, 3); return selected.first() as number; }",
    )
    .expect("compile Array slice");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array instructions must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(7.0, zun_il::NumKind::F64)));
}

#[test]
fn array_concat_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { concat(other: Array): Array { let result = new Array(this.length + other.length); let index: u64 = 0; while (index < this.length) { result[index] = this[index]; index = index + 1; } let otherIndex: u64 = 0; while (otherIndex < other.length) { result[index + otherIndex] = other[otherIndex]; otherIndex = otherIndex + 1; } return result; } first(): any { return this[0]; } }\n\
         function main(): number { let left = new Array(1); left[0] = 7.0; let right = new Array(1); right[0] = 9.0; let joined = left.concat(right); return joined[1] as number; }",
    )
    .expect("compile Array concat");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array instructions must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(9.0, zun_il::NumKind::F64)));
}

#[test]
fn array_operations_use_interpreter_fallback_until_array_jit_lowering_exists() {
    let module = zun_compiler::compile(
        "function main(): any { let values = new Array(1); values[0] = 41.0; return values[0]; }",
    )
    .expect("compile array program");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with array fallback");
    assert!(!jit.is_compiled(entry), "array instructions must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 41.0));
}

#[test]
fn unsigned_wrap_u8() {
    assert_equivalent(
        "function main(): u8 { let b: u8 = 250; return b + 10 as u8; }",
    );
}

#[test]
fn signed_overflow_i8() {
    assert_equivalent(
        "function main(): i8 { let s: i8 = 127; return s + 1 as i8; }",
    );
}

#[test]
fn unsigned_division_u32() {
    assert_equivalent(
        "function main(): u32 { let x: u32 = 4000000000 as u32; return x / 2 as u32; }",
    );
}

#[test]
fn unsigned_comparison() {
    // 4e9 as u32 is > 1 unsigned but would be negative as i32; the JIT must use
    // an unsigned compare to match the interpreter.
    assert_equivalent(
        "function main(): int {\n\
            let x: u32 = 4000000000 as u32;\n\
            if (x > 1 as u32) { return 1; }\n\
            return 0;\n\
         }",
    );
}

#[test]
fn f32_precision() {
    assert_equivalent(
        "function main(): f32 { let x: f32 = 1.0 as f32; return x / 3.0 as f32; }",
    );
}

#[test]
fn int_to_float_conversion() {
    assert_equivalent(
        "function main(): number { let n: int = 7; return n as number / 2.0; }",
    );
}

#[test]
fn i64_arithmetic() {
    assert_equivalent(
        "function main(): i64 { let x: i64 = 1000000 as i64; return x * x; }",
    );
}

#[test]
fn boolean_return() {
    assert_equivalent(
        "function main(): boolean { let a: int = 3; return a < 5; }",
    );
}

#[test]
fn object_alloc_and_field_access_in_jit() {
    // A helper that builds a Point, writes/reads fields, and returns a number.
    // It uses newobj/ldfld/stfld with numeric fields, so it is JIT-eligible;
    // the result must match the interpreter.
    let src = "class Point { x: int; y: int; }\n\
               function dist2(): int {\n\
                   let p: Point = new Point(3, 4);\n\
                   p.x = p.x + 1;\n\
                   return p.x * p.x + p.y * p.y;\n\
               }\n\
               function main(): int { return dist2(); }";
    let module = compile(src);
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    // dist2 (rid 2) uses objects and must be JIT-eligible now.
    assert!(jit.is_compiled(2), "object method should be eligible");

    // The JIT needs a current heap for newobj; the VM sets one per run, so a
    // standalone call must register one too.
    let mut heap = zun_gc::Heap::new();
    let jit_result = {
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    let matches = match (&jit_result, &interp_result) {
        (RetVal::Int(a), Value::Int(b, _)) => a == b,
        _ => false,
    };
    assert!(matches, "JIT {jit_result:?} vs interp {interp_result:?}");
}

#[test]
fn non_overridden_virtual_call_is_jit_eligible() {
    let src = "class Counter { n: int; get(): int { return this.n; } }\n\
               function read(c: Counter): int { return c.get(); }\n\
               function main(): int { return read(new Counter(7)); }";
    let module = compile(src);
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 7));
}

#[test]
fn aot_object_contains_the_eligible_entry_point() {
    let module = compile("function main(): int { return 7; }");
    let object = zun_codegen::compile_object(&module).expect("emit object");
    assert!(!object.is_empty());
    assert!(object.windows(2).any(|bytes| bytes == b"m1"));
}

#[test]
fn overridden_virtual_call_is_jit_eligible_and_dispatches_dynamically() {
    let src = "class Base { speak(): int { return 1; } }\n\
               class Derived extends Base { speak(): int { return 2; } }\n\
               function read(b: Base): int { return b.speak(); }\n\
               function main(): int { return read(new Derived()); }";
    let module = compile(src);
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit compiles");
    assert!(jit.is_compiled(entry));
    let jit_result = {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        jit.call_entry(entry).expect("jit call")
    };
    let interp_result = interp_entry(&module);
    assert!(matches!((jit_result, interp_result),
        (RetVal::Int(a), Value::Int(b, _)) if a == b && a == 2));
}

#[test]
fn string_field_method_is_not_jit_eligible() {
    // A method touching a string field must stay on the interpreter.
    let src = "class Named { tag: string; }\n\
               function make(): int { let n: Named = new Named(\"hi\"); return 1; }\n\
               function main(): int { return make(); }";
    let module = compile(src);
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(2), "string-field method must be ineligible");
}

#[test]
fn ineligible_methods_are_not_compiled() {
    let module = compile(r#"function main() { print("hi"); }"#);
    let entry = module.entry_point.unwrap().rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry));
}
