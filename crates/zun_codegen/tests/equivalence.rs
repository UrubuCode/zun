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
fn managed_error_hierarchy_constructs_and_preserves_fields() {
    let module = compile(
        "class Error { name: string; message: string; }\n\
         class TypeError extends Error {}\n\
         function main(): int { let error: TypeError = new TypeError(\"TypeError\", \"bad value\"); if (error.name == \"TypeError\") { if (error.message == \"bad value\") { return 1; } } return 0; }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::new(&module).expect("jit builds");
    assert!(!jit.is_compiled(entry), "string-field error objects stay interpreter-only");
    assert!(matches!(interp_entry(&module), Value::Int(1, _)));
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
fn string_includes_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { includes } from \"zun:string\";\n\
         class String { extern includes(needle: string): boolean; }\n\
         function main(): boolean { let value: string = \"zun runtime\"; return value.includes(\"runtime\"); }",
    )
    .expect("compile String includes");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
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
fn string_case_methods_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { toLowerCase, toUpperCase } from \"zun:string\";\n\
         class String { extern toLowerCase(): string; extern toUpperCase(): string; }\n\
         function main(): string { let value: string = \"ZuN\"; return value.toLowerCase() + value.toUpperCase(); }",
    )
    .expect("compile String case methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zunZUN"));
}

#[test]
fn string_trim_boundary_methods_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { trimStart, trimEnd } from \"zun:string\";\n\
         class String { extern trimStart(): string; extern trimEnd(): string; }\n\
         function main(): string { let value: string = \"  zun  \"; return value.trimStart().trimEnd(); }",
    )
    .expect("compile String trim boundary methods");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "zun"));
}

#[test]
fn string_is_blank_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { trim, length } from \"zun:string\";\n\
         class String { extern trim(): string; extern length(): u64; isEmpty(): boolean { return this.length() == 0; } isBlank(): boolean { return this.trim().isEmpty(); } }\n\
         function main(): boolean { let value: string = \"   \"; return value.isBlank(); }",
    )
    .expect("compile String isBlank");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_not_empty_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length } from \"zun:string\";\n\
         class String { extern length(): u64; isNotEmpty(): boolean { return this.length() != 0; } }\n\
         function main(): boolean { let value: string = \"zun\"; return value.isNotEmpty(); }",
    )
    .expect("compile String isNotEmpty");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_reverse_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { reverse } from \"zun:string\";\n\
         class String { extern reverse(): string; }\n\
         function main(): string { let value: string = \"zun\"; return value.reverse(); }",
    )
    .expect("compile String reverse");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value.as_ref() == "nuz"));
}

#[test]
fn string_is_ascii_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\";\n\
         class String { extern length(): u64; extern charCodeAt(index: u64): u32; isAscii(): boolean { let index: u64 = 0; while (index < this.length()) { if (this.charCodeAt(index) > 127) { return false; } index = index + 1; } return true; } }\n\
         function main(): boolean { let value: string = \"zun\"; return value.isAscii(); }",
    )
    .expect("compile String isAscii");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_count_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { count } from \"zun:string\";\n\
         class String { extern count(needle: string): int; }\n\
         function main(): int { let value: string = \"aaaa\"; return value.count(\"aa\"); }",
    )
    .expect("compile String count");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 2));
}

#[test]
fn string_is_numeric_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\";\n\
         class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isNumeric(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } }\n\
         function main(): boolean { let value: string = \"12345\"; return value.isNumeric(); }",
    )
    .expect("compile String isNumeric");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_compare_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { compare } from \"zun:string\";\n\
         class String { extern compare(other: string): int; }\n\
         function main(): int { let value: string = \"abc\"; return value.compare(\"abd\"); }",
    )
    .expect("compile String compare");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == -1));
}

#[test]
fn string_is_whitespace_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, trim } from \"zun:string\";\n\
         class String { extern length(): u64; extern trim(): string; isEmpty(): boolean { return this.length() == 0; } isWhitespace(): boolean { if (this.isEmpty()) { return false; } return this.trim().isEmpty(); } }\n\
         function main(): boolean { let value: string = \"   \"; return value.isWhitespace(); }",
    )
    .expect("compile String isWhitespace");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_alpha_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\";\n\
         class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isAlpha(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } index = index + 1; } return true; } }\n\
         function main(): boolean { let value: string = \"ZunRuntime\"; return value.isAlpha(); }",
    )
    .expect("compile String isAlpha");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_alphanumeric_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\";\n\
         class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isAlphanumeric(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } } index = index + 1; } return true; } }\n\
         function main(): boolean { let value: string = \"Zun2026\"; return value.isAlphanumeric(); }",
    )
    .expect("compile String isAlphanumeric");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_case_predicates_use_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\";\n\
         class String { extern length(): u64; extern charCodeAt(index: u64): u32; isLowerCase(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code >= 65) { if (code <= 90) { return false; } if (code >= 97) { if (code <= 122) { hasLetter = true; } } } index = index + 1; } return hasLetter; } isUpperCase(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code >= 97) { if (code <= 122) { return false; } } if (code >= 65) { if (code <= 90) { hasLetter = true; } } index = index + 1; } return hasLetter; } }\n\
         function main(): boolean { let lower: string = \"hello\"; let upper: string = \"WORLD\"; if (!lower.isLowerCase()) { return false; } return upper.isUpperCase(); }",
    )
    .expect("compile String case predicates");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_to_int_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { toInt } from \"zun:string\";\n\
         class String { extern toInt(): int; }\n\
         function main(): int { let value: string = \" -42 \"; return value.toInt(); }",
    )
    .expect("compile String toInt");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == -42));
}

#[test]
fn string_to_float_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { toFloat } from \"zun:string\";\n\
         class String { extern toFloat(): number; }\n\
         function main(): number { let value: string = \" 3.5 \"; return value.toFloat(); }",
    )
    .expect("compile String toFloat");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 3.5));
}

#[test]
fn string_to_boolean_uses_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { toBoolean } from \"zun:string\";\n\
         class String { extern toBoolean(): boolean; }\n\
         function main(): boolean { let value: string = \" true \"; return value.toBoolean(); }",
    )
    .expect("compile String toBoolean");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
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
        "class Array { indexOf(value: any): int { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return index as int; } index = index + 1; } return -1; } findIndex(value: any): int { return this.indexOf(value); } }\n\
         function main(): int { let values = new Array(3); values[0] = 4; values[1] = 8; values[2] = 15; let needle: any = values[1]; return values.findIndex(needle); }",
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

#[test]
fn string_is_palindrome_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { reverse } from \"zun:string\"; class String { extern reverse(): string; isPalindrome(): boolean { return this == this.reverse(); } } function main(): boolean { let value: string = \"level\"; return value.isPalindrome(); }",
    )
    .expect("compile String isPalindrome");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_hexadecimal_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isHexadecimal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { if (code < 65) { return false; } if (code > 70) { if (code < 97) { return false; } if (code > 102) { return false; } } } index = index + 1; } return true; } } function main(): boolean { let value: string = \"a1F0\"; return value.isHexadecimal(); }",
    )
    .expect("compile String isHexadecimal");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_hex_digit_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isHexDigit(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 70) { valid = true; } } if (code >= 97) { if (code <= 102) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"aF09\"; return value.isHexDigit(); }",
    )
    .expect("compile String isHexDigit");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_binary_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isBinary(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 49) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"101001\"; return value.isBinary(); }",
    )
    .expect("compile String isBinary");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_octal_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isOctal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 55) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"701234\"; return value.isOctal(); }",
    )
    .expect("compile String isOctal");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_printable_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPrintable(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 32) { return false; } if (code > 126) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"Zun 2026!\"; return value.isPrintable(); }",
    )
    .expect("compile String isPrintable");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_punctuation_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPunctuation(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let punctuation: boolean = false; if (code >= 33) { if (code <= 47) { punctuation = true; } } if (code >= 58) { if (code <= 64) { punctuation = true; } } if (code >= 91) { if (code <= 96) { punctuation = true; } } if (code >= 123) { if (code <= 126) { punctuation = true; } } if (!punctuation) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"!?.,\"; return value.isPunctuation(); }",
    )
    .expect("compile String isPunctuation");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_digit_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isDigit(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"2048\"; return value.isDigit(); }",
    )
    .expect("compile String isDigit");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_vowel_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isVowel(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let vowel: boolean = false; if (code == 65) { vowel = true; } if (code == 69) { vowel = true; } if (code == 73) { vowel = true; } if (code == 79) { vowel = true; } if (code == 85) { vowel = true; } if (code == 97) { vowel = true; } if (code == 101) { vowel = true; } if (code == 105) { vowel = true; } if (code == 111) { vowel = true; } if (code == 117) { vowel = true; } if (!vowel) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"AeIOu\"; return value.isVowel(); }",
    )
    .expect("compile String isVowel");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_consonant_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isAlpha(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } index = index + 1; } return true; } isConsonant(): boolean { if (!this.isAlpha()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 65) { return false; } if (code == 69) { return false; } if (code == 73) { return false; } if (code == 79) { return false; } if (code == 85) { return false; } if (code == 97) { return false; } if (code == 101) { return false; } if (code == 105) { return false; } if (code == 111) { return false; } if (code == 117) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"Rhythm\"; return value.isConsonant(); }",
    )
    .expect("compile String isConsonant");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_quoted_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isQuoted(): boolean { if (this.length() < 2) { return false; } let first: u32 = this.charCodeAt(0); let last: u32 = this.charCodeAt(this.length() - 1); if (first == 34) { return last == 34; } if (first == 39) { return last == 39; } return false; } } function main(): boolean { let value: string = \"\\\"zun\\\"\"; return value.isQuoted(); }",
    )
    .expect("compile String isQuoted");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_unquote_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt, substring } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; extern substring(start: u64, end: u64): string; isQuoted(): boolean { if (this.length() < 2) { return false; } let first: u32 = this.charCodeAt(0); let last: u32 = this.charCodeAt(this.length() - 1); if (first == 34) { return last == 34; } if (first == 39) { return last == 39; } return false; } unquote(): string { if (!this.isQuoted()) { return this; } return this.substring(1, this.length() - 1); } } function main(): string { let value: string = \"\\\"zun\\\"\"; return value.unquote(); }",
    )
    .expect("compile String unquote");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value == "zun".into()));
}

#[test]
fn string_is_wrapped_by_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { startsWith, endsWith } from \"zun:string\"; class String { extern startsWith(prefix: string): boolean; extern endsWith(suffix: string): boolean; isWrappedBy(prefix: string, suffix: string): boolean { if (!this.startsWith(prefix)) { return false; } return this.endsWith(suffix); } } function main(): boolean { let value: string = \"<zun>\"; return value.isWrappedBy(\"<\", \">\"); }",
    )
    .expect("compile String isWrappedBy");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_quote_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class String { quote(): string { return \"\\\"\" + this + \"\\\"\"; } } function main(): string { let value: string = \"zun\"; return value.quote(); }",
    )
    .expect("compile String quote");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Str(value) if value == "\"zun\"".into()));
}

#[test]
fn string_is_control_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isControl(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code > 31) { if (code != 127) { return false; } } index = index + 1; } return true; } } function main(): boolean { let value: string = \"\\n\\t\"; return value.isControl(); }",
    )
    .expect("compile String isControl");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_signed_integer_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSignedInteger(): boolean { if (this.isEmpty()) { return false; } let start: u64 = 0; let first: u32 = this.charCodeAt(0); if (first == 43) { start = 1; } if (first == 45) { start = 1; } if (start == this.length()) { return false; } let index: u64 = start; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"-2048\"; return value.isSignedInteger(); }",
    )
    .expect("compile String isSignedInteger");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_signed_decimal_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSignedDecimal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let digits: u64 = 0; let dots: u64 = 0; let first: u32 = this.charCodeAt(0); if (first == 43) { index = 1; } if (first == 45) { index = 1; } while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 46) { dots = dots + 1; if (dots > 1) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } digits = digits + 1; } index = index + 1; } return digits > 0; } } function main(): boolean { let value: string = \"-12.50\"; return value.isSignedDecimal(); }",
    )
    .expect("compile String isSignedDecimal");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_identifier_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isIdentifier(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code == 95) { valid = true; } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (index > 0) { if (code >= 48) { if (code <= 57) { valid = true; } } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"zun_core2\"; return value.isIdentifier(); }",
    )
    .expect("compile String isIdentifier");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_title_case_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isTitleCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let expectUpper: boolean = true; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 32) { expectUpper = true; } else { if (expectUpper) { if (code < 65) { return false; } if (code > 90) { return false; } expectUpper = false; hasLetter = true; } else { if (code >= 65) { if (code <= 90) { return false; } } if (code < 97) { return false; } if (code > 122) { return false; } } } index = index + 1; } if (!hasLetter) { return false; } return !expectUpper; } } function main(): boolean { let value: string = \"Hello World\"; return value.isTitleCase(); }",
    )
    .expect("compile String isTitleCase");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_snake_case_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSnakeCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let previousSeparator: boolean = true; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 95) { if (previousSeparator) { return false; } previousSeparator = true; } else { if (code >= 97) { if (code > 122) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } } previousSeparator = false; } index = index + 1; } return !previousSeparator; } } function main(): boolean { let value: string = \"zun_runtime2\"; return value.isSnakeCase(); }",
    )
    .expect("compile String isSnakeCase");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_kebab_case_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isKebabCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let previousSeparator: boolean = true; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 45) { if (previousSeparator) { return false; } previousSeparator = true; } else { if (code >= 97) { if (code > 122) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } } previousSeparator = false; } index = index + 1; } return !previousSeparator; } } function main(): boolean { let value: string = \"zun-runtime2\"; return value.isKebabCase(); }",
    )
    .expect("compile String isKebabCase");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_camel_case_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isCamelCase(): boolean { if (this.isEmpty()) { return false; } let first: u32 = this.charCodeAt(0); if (first < 97) { return false; } if (first > 122) { return false; } let index: u64 = 1; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"zunRuntime2\"; return value.isCamelCase(); }",
    )
    .expect("compile String isCamelCase");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn string_is_pascal_case_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPascalCase(): boolean { if (this.isEmpty()) { return false; } let first: u32 = this.charCodeAt(0); if (first < 65) { return false; } if (first > 90) { return false; } let index: u64 = 1; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"ZunRuntime2\"; return value.isPascalCase(); }",
    )
    .expect("compile String isPascalCase");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with String fallback");
    assert!(!jit.is_compiled(entry), "String methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_sign_predicates_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isPositive(): boolean { return this > 0.0; } isNonZero(): boolean { return this != 0.0; } } function main(): boolean { let value: number = 3.0; if (!value.isPositive()) { return false; } return value.isNonZero(); }",
    )
    .expect("compile Number sign predicates");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_is_between_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isBetween(low: number, high: number): boolean { if (this < low) { return false; } return this <= high; } } function main(): boolean { let value: number = 5.0; return value.isBetween(1.0, 10.0); }",
    )
    .expect("compile Number isBetween");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_is_in_unit_interval_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isBetween(low: number, high: number): boolean { if (this < low) { return false; } return this <= high; } isInUnitInterval(): boolean { return this.isBetween(0.0, 1.0); } } function main(): boolean { let value: number = 0.75; return value.isInUnitInterval(); }",
    )
    .expect("compile Number isInUnitInterval");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_is_between_exclusive_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isBetweenExclusive(low: number, high: number): boolean { if (this <= low) { return false; } return this < high; } } function main(): boolean { let value: number = 5.0; return value.isBetweenExclusive(0.0, 10.0); }",
    )
    .expect("compile Number isBetweenExclusive");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_distance_to_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { abs(): number { if (this < 0.0) { return -this; } return this; } distanceTo(other: number): number { return (this - other).abs(); } } function main(): number { let value: number = 3.0; return value.distanceTo(10.5); }",
    )
    .expect("compile Number distanceTo");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 7.5));
}

#[test]
fn number_inverse_lerp_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { inverseLerp(low: number, high: number): number { if (low == high) { return 0.0; } return (this - low) / (high - low); } } function main(): number { let value: number = 25.0; return value.inverseLerp(0.0, 100.0); }",
    )
    .expect("compile Number inverseLerp");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 0.25));
}

#[test]
fn number_smooth_step_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { clamp(low: number, high: number): number { if (this < low) { return low; } if (this > high) { return high; } return this; } inverseLerp(low: number, high: number): number { if (low == high) { return 0.0; } return (this - low) / (high - low); } smoothStep(edge0: number, edge1: number): number { let amount: number = this.inverseLerp(edge0, edge1).clamp(0.0, 1.0); return amount * amount * (3.0 - 2.0 * amount); } } function main(): number { let value: number = 50.0; return value.smoothStep(0.0, 100.0); }",
    )
    .expect("compile Number smoothStep");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 0.5));
}

#[test]
fn number_is_approximately_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { isApproximately(other: number, epsilon: number): boolean { let difference: number = this - other; if (difference < 0.0) { difference = -difference; } return difference <= epsilon; } } function main(): boolean { let value: number = 1.001; return value.isApproximately(1.0, 0.01); }",
    )
    .expect("compile Number isApproximately");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn number_lerp_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { lerp(target: number, amount: number): number { return this + (target - this) * amount; } } function main(): number { let value: number = 10.0; return value.lerp(20.0, 0.25); }",
    )
    .expect("compile Number lerp");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 12.5));
}

#[test]
fn number_remap_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Number { remap(fromLow: number, fromHigh: number, toLow: number, toHigh: number): number { if (fromLow == fromHigh) { return toLow; } let amount: number = (this - fromLow) / (fromHigh - fromLow); return toLow + (toHigh - toLow) * amount; } } function main(): number { let value: number = 5.0; return value.remap(0.0, 10.0, 100.0, 200.0); }",
    )
    .expect("compile Number remap");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Number fallback");
    assert!(!jit.is_compiled(entry), "Number methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 150.0));
}

#[test]
fn math_power_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { powNumber } from \"zun:math\"; function main(): number { return powNumber(2.0, 3.0); }",
    )
    .expect("compile math power");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math fallback");
    assert!(!jit.is_compiled(entry), "native math calls must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 8.0));
}

#[test]
fn math_trigonometry_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { sinNumber, cosNumber } from \"zun:math\"; function main(): number { return sinNumber(0.0) + cosNumber(0.0); }",
    )
    .expect("compile math trigonometry");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math fallback");
    assert!(!jit.is_compiled(entry), "native math calls must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 1.0));
}

#[test]
fn math_exponential_and_logarithm_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { expNumber, logNumber } from \"zun:math\"; function main(): number { return expNumber(0.0) + logNumber(1.0); }",
    )
    .expect("compile math exponential and logarithm");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math fallback");
    assert!(!jit.is_compiled(entry), "native math calls must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 1.0));
}

#[test]
fn math_tangent_and_atan2_keep_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { tanNumber, atan2Number } from \"zun:math\"; function main(): number { return tanNumber(0.0) + atan2Number(0.0, 1.0); }",
    )
    .expect("compile math tangent and atan2");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math fallback");
    assert!(!jit.is_compiled(entry), "native math calls must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 0.0));
}

#[test]
fn math_hypot_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { hypotNumber } from \"zun:math\"; function main(): number { return hypotNumber(3.0, 4.0); }",
    )
    .expect("compile math hypot");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math fallback");
    assert!(!jit.is_compiled(entry), "native math calls must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 5.0));
}

#[test]
fn break_keeps_interpreter_and_jit_paths_equivalent() {
    let module = zun_compiler::compile(
        "function main(): u64 { let index: u64 = 0; while (index < 10) { if (index == 3) { break; } index = index + 1; } return index; }",
    )
    .expect("compile break");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds break module");
    assert!(jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 3));
    assert!(matches!(jit.call_entry(entry).expect("run JIT break"), RetVal::Int(3)));
}

#[test]
fn continue_keeps_interpreter_and_jit_paths_equivalent() {
    let module = zun_compiler::compile(
        "function main(): u64 { let index: u64 = 0; let total: u64 = 0; while (index < 5) { index = index + 1; if (index == 3) { continue; } total = total + index; } return total; }",
    )
    .expect("compile continue");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds continue module");
    assert!(jit.is_compiled(entry));
    assert!(matches!(interp_entry(&module), Value::Int(value, _) if value == 12));
    assert!(matches!(jit.call_entry(entry).expect("run JIT continue"), RetVal::Int(12)));
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
fn array_copy_within_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { slice(start: u64, end: u64): Array { let count: u64 = end - start; let result = new Array(count); let index: u64 = 0; while (index < count) { result[index] = this[start + index]; index = index + 1; } return result; } copyWithin(target: u64, start: u64, end: u64): Array { let copied = this.slice(start, end); let index: u64 = 0; while (index < copied.length) { this[target + index] = copied[index]; index = index + 1; } return this; } }\n\
         function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 7.0; values[2] = 9.0; values.copyWithin(0, 1, 3); return values[0] as number; }",
    )
    .expect("compile Array copyWithin");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array instructions must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(7.0, zun_il::NumKind::F64)));
}

#[test]
fn array_flat_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { flat(): Array { let total: u64 = 0; let outer: u64 = 0; while (outer < this.length) { let nested: Array = this[outer] as Array; total = total + nested.length; outer = outer + 1; } let result = new Array(total); let output: u64 = 0; outer = 0; while (outer < this.length) { let nested: Array = this[outer] as Array; let inner: u64 = 0; while (inner < nested.length) { result[output] = nested[inner]; output = output + 1; inner = inner + 1; } outer = outer + 1; } return result; } }\n\
         function main(): number { let left = new Array(1); left[0] = 7.0; let right = new Array(1); right[0] = 9.0; let outer = new Array(2); outer[0] = left; outer[1] = right; let flattened = outer.flat(); return flattened[1] as number; }",
    )
    .expect("compile Array flat");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array instructions must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(9.0, zun_il::NumKind::F64)));
}

#[test]
fn array_includes_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } includes(value: any): boolean { return this.contains(value); } }\n\
         function main(): boolean { let values = new Array(1); values[0] = 41; return values.includes(values[0]); }",
    )
    .expect("compile Array includes");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array instructions must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_all_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { all(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] != value) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 4.0; values[1] = 4.0; values[2] = 4.0; let needle: any = values[0]; return values.all(needle); }",
    )
    .expect("compile Array all");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_none_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { none(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; let needle: any = values[0]; return values.none(needle); }",
    )
    .expect("compile Array none");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(false)));
}

#[test]
fn array_any_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { any(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; let needle: any = values[1]; return values.any(needle); }",
    )
    .expect("compile Array any");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_constant_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isConstant(): boolean { if (this.length < 2) { return true; } let first: any = this[0]; let index: u64 = 1; while (index < this.length) { if (this[index] != first) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 9.0; values[1] = 9.0; values[2] = 9.0; return values.isConstant(); }",
    )
    .expect("compile Array isConstant");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_palindrome_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isPalindrome(): boolean { if (this.length < 2) { return true; } let left: u64 = 0; let right: u64 = this.length - 1; while (left < right) { if (this[left] != this[right]) { return false; } left = left + 1; right = right - 1; } return true; } } function main(): boolean { let values = new Array(5); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values[3] = 2.0; values[4] = 1.0; return values.isPalindrome(); }",
    )
    .expect("compile Array isPalindrome");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_disjoint_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { isDisjoint(other: Array): boolean { let left: u64 = 0; while (left < this.length) { let right: u64 = 0; while (right < other.length) { if (this[left] == other[right]) { return false; } right = right + 1; } left = left + 1; } return true; } } function main(): boolean { let left = new Array(2); left[0] = 1.0; left[1] = 2.0; let right = new Array(2); right[0] = 3.0; right[1] = 4.0; return left.isDisjoint(right); }",
    )
    .expect("compile Array isDisjoint");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_intersection_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { intersection(other: Array): Array { let matches: u64 = 0; let left: u64 = 0; while (left < this.length) { let right: u64 = 0; while (right < other.length) { if (this[left] == other[right]) { matches = matches + 1; break; } right = right + 1; } left = left + 1; } let result = new Array(matches); let output: u64 = 0; left = 0; while (left < this.length) { let right: u64 = 0; while (right < other.length) { if (this[left] == other[right]) { result[output] = this[left]; output = output + 1; break; } right = right + 1; } left = left + 1; } return result; } } function main(): number { let left = new Array(3); left[0] = 1.0; left[1] = 2.0; left[2] = 3.0; let right = new Array(2); right[0] = 2.0; right[1] = 4.0; let common = left.intersection(right); return common[0] as number; }",
    )
    .expect("compile Array intersection");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 2.0));
}

#[test]
fn array_difference_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { difference(other: Array): Array { let remaining: u64 = 0; let left: u64 = 0; while (left < this.length) { let right: u64 = 0; let found: boolean = false; while (right < other.length) { if (this[left] == other[right]) { found = true; break; } right = right + 1; } if (!found) { remaining = remaining + 1; } left = left + 1; } let result = new Array(remaining); let output: u64 = 0; left = 0; while (left < this.length) { let right: u64 = 0; let found: boolean = false; while (right < other.length) { if (this[left] == other[right]) { found = true; break; } right = right + 1; } if (!found) { result[output] = this[left]; output = output + 1; } left = left + 1; } return result; } } function main(): number { let left = new Array(3); left[0] = 1.0; left[1] = 2.0; left[2] = 3.0; let right = new Array(2); right[0] = 2.0; right[1] = 4.0; let remaining = left.difference(right); return remaining[1] as number; }",
    )
    .expect("compile Array difference");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 3.0));
}

#[test]
fn array_union_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } union(other: Array): Array { let total: u64 = this.length; let right: u64 = 0; while (right < other.length) { if (!this.contains(other[right])) { total = total + 1; } right = right + 1; } let result = new Array(total); let index: u64 = 0; while (index < this.length) { result[index] = this[index]; index = index + 1; } right = 0; while (right < other.length) { if (!this.contains(other[right])) { result[index] = other[right]; index = index + 1; } right = right + 1; } return result; } } function main(): number { let left = new Array(2); left[0] = 1.0; left[1] = 2.0; let right = new Array(2); right[0] = 2.0; right[1] = 3.0; let combined = left.union(right); return combined[2] as number; }",
    )
    .expect("compile Array union");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 3.0));
}

#[test]
fn array_symmetric_difference_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } difference(other: Array): Array { let remaining: u64 = 0; let left: u64 = 0; while (left < this.length) { if (!other.contains(this[left])) { remaining = remaining + 1; } left = left + 1; } let result = new Array(remaining); let output: u64 = 0; left = 0; while (left < this.length) { if (!other.contains(this[left])) { result[output] = this[left]; output = output + 1; } left = left + 1; } return result; } union(other: Array): Array { let total: u64 = this.length; let right: u64 = 0; while (right < other.length) { if (!this.contains(other[right])) { total = total + 1; } right = right + 1; } let result = new Array(total); let index: u64 = 0; while (index < this.length) { result[index] = this[index]; index = index + 1; } right = 0; while (right < other.length) { if (!this.contains(other[right])) { result[index] = other[right]; index = index + 1; } right = right + 1; } return result; } symmetricDifference(other: Array): Array { let leftOnly = this.difference(other); let rightOnly = other.difference(this); return leftOnly.union(rightOnly); } } function main(): number { let left = new Array(2); left[0] = 1.0; left[1] = 2.0; let right = new Array(2); right[0] = 2.0; right[1] = 3.0; let result = left.symmetricDifference(right); return result[1] as number; }",
    )
    .expect("compile Array symmetricDifference");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 3.0));
}

#[test]
fn array_is_subset_of_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } isSubsetOf(other: Array): boolean { let index: u64 = 0; while (index < this.length) { if (!other.contains(this[index])) { return false; } index = index + 1; } return true; } } function main(): boolean { let subset = new Array(2); subset[0] = 1.0; subset[1] = 2.0; let full = new Array(3); full[0] = 1.0; full[1] = 2.0; full[2] = 3.0; return subset.isSubsetOf(full); }",
    )
    .expect("compile Array isSubsetOf");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
}

#[test]
fn array_is_superset_of_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } isSubsetOf(other: Array): boolean { let index: u64 = 0; while (index < this.length) { if (!other.contains(this[index])) { return false; } index = index + 1; } return true; } isSupersetOf(other: Array): boolean { return other.isSubsetOf(this); } } function main(): boolean { let subset = new Array(2); subset[0] = 1.0; subset[1] = 2.0; let full = new Array(3); full[0] = 1.0; full[1] = 2.0; full[2] = 3.0; return full.isSupersetOf(subset); }",
    )
    .expect("compile Array isSupersetOf");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with Array fallback");
    assert!(!jit.is_compiled(entry), "Array methods must remain Tier 0");
    assert!(matches!(interp_entry(&module), Value::Bool(true)));
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
