mod common;
use common::*;

use zun_codegen::RetVal;
use zun_interp::value::Value;

#[test]
fn string_is_palindrome_keeps_interpreter_fallback() {
    let module = zun_compiler::compile(
        "import { reverse } from \"zun:core\"; class String { extern reverse(): string; isPalindrome(): boolean { return this == this.reverse(); } } function main(): boolean { let value: string = \"level\"; return value.isPalindrome(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isHexadecimal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { if (code < 65) { return false; } if (code > 70) { if (code < 97) { return false; } if (code > 102) { return false; } } } index = index + 1; } return true; } } function main(): boolean { let value: string = \"a1F0\"; return value.isHexadecimal(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isHexDigit(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 70) { valid = true; } } if (code >= 97) { if (code <= 102) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"aF09\"; return value.isHexDigit(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isBinary(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 49) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"101001\"; return value.isBinary(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isOctal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 55) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"701234\"; return value.isOctal(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPrintable(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 32) { return false; } if (code > 126) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"Zun 2026!\"; return value.isPrintable(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPunctuation(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let punctuation: boolean = false; if (code >= 33) { if (code <= 47) { punctuation = true; } } if (code >= 58) { if (code <= 64) { punctuation = true; } } if (code >= 91) { if (code <= 96) { punctuation = true; } } if (code >= 123) { if (code <= 126) { punctuation = true; } } if (!punctuation) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"!?.,\"; return value.isPunctuation(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isDigit(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"2048\"; return value.isDigit(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isVowel(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let vowel: boolean = false; if (code == 65) { vowel = true; } if (code == 69) { vowel = true; } if (code == 73) { vowel = true; } if (code == 79) { vowel = true; } if (code == 85) { vowel = true; } if (code == 97) { vowel = true; } if (code == 101) { vowel = true; } if (code == 105) { vowel = true; } if (code == 111) { vowel = true; } if (code == 117) { vowel = true; } if (!vowel) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"AeIOu\"; return value.isVowel(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isAlpha(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } index = index + 1; } return true; } isConsonant(): boolean { if (!this.isAlpha()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 65) { return false; } if (code == 69) { return false; } if (code == 73) { return false; } if (code == 79) { return false; } if (code == 85) { return false; } if (code == 97) { return false; } if (code == 101) { return false; } if (code == 105) { return false; } if (code == 111) { return false; } if (code == 117) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"Rhythm\"; return value.isConsonant(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isQuoted(): boolean { if (this.length() < 2) { return false; } let zeroIndex: u64 = 0; let first: u32 = this.charCodeAt(zeroIndex); let last: u32 = this.charCodeAt(this.length() - 1); if (first == 34) { return last == 34; } if (first == 39) { return last == 39; } return false; } } function main(): boolean { let value: string = \"\\\"zun\\\"\"; return value.isQuoted(); }",
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
        "import { length, charCodeAt, substring } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; extern substring(start: u64, end: u64): string; isQuoted(): boolean { if (this.length() < 2) { return false; } let zeroIndex: u64 = 0; let first: u32 = this.charCodeAt(zeroIndex); let last: u32 = this.charCodeAt(this.length() - 1); if (first == 34) { return last == 34; } if (first == 39) { return last == 39; } return false; } unquote(): string { if (!this.isQuoted()) { return this; } return this.substring(1, this.length() - 1); } } function main(): string { let value: string = \"\\\"zun\\\"\"; return value.unquote(); }",
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
        "import { startsWith, endsWith } from \"zun:core\"; class String { extern startsWith(prefix: string): boolean; extern endsWith(suffix: string): boolean; isWrappedBy(prefix: string, suffix: string): boolean { if (!this.startsWith(prefix)) { return false; } return this.endsWith(suffix); } } function main(): boolean { let value: string = \"<zun>\"; return value.isWrappedBy(\"<\", \">\"); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isControl(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code > 31) { if (code != 127) { return false; } } index = index + 1; } return true; } } function main(): boolean { let value: string = \"\\n\\t\"; return value.isControl(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSignedInteger(): boolean { if (this.isEmpty()) { return false; } let start: u64 = 0; let zeroIndex: u64 = 0; let first: u32 = this.charCodeAt(zeroIndex); if (first == 43) { start = 1; } if (first == 45) { start = 1; } if (start == this.length()) { return false; } let index: u64 = start; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"-2048\"; return value.isSignedInteger(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSignedDecimal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let digits: u64 = 0; let dots: u64 = 0; let zeroIndex: u64 = 0; let first: u32 = this.charCodeAt(zeroIndex); if (first == 43) { index = 1; } if (first == 45) { index = 1; } while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 46) { dots = dots + 1; if (dots > 1) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } digits = digits + 1; } index = index + 1; } return digits > 0; } } function main(): boolean { let value: string = \"-12.50\"; return value.isSignedDecimal(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isIdentifier(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code == 95) { valid = true; } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (index > 0) { if (code >= 48) { if (code <= 57) { valid = true; } } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"zun_core2\"; return value.isIdentifier(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isTitleCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let expectUpper: boolean = true; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 32) { expectUpper = true; } else { if (expectUpper) { if (code < 65) { return false; } if (code > 90) { return false; } expectUpper = false; hasLetter = true; } else { if (code >= 65) { if (code <= 90) { return false; } } if (code < 97) { return false; } if (code > 122) { return false; } } } index = index + 1; } if (!hasLetter) { return false; } return !expectUpper; } } function main(): boolean { let value: string = \"Hello World\"; return value.isTitleCase(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSnakeCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let previousSeparator: boolean = true; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 95) { if (previousSeparator) { return false; } previousSeparator = true; } else { if (code >= 97) { if (code > 122) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } } previousSeparator = false; } index = index + 1; } return !previousSeparator; } } function main(): boolean { let value: string = \"zun_runtime2\"; return value.isSnakeCase(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isKebabCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let previousSeparator: boolean = true; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 45) { if (previousSeparator) { return false; } previousSeparator = true; } else { if (code >= 97) { if (code > 122) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } } previousSeparator = false; } index = index + 1; } return !previousSeparator; } } function main(): boolean { let value: string = \"zun-runtime2\"; return value.isKebabCase(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isCamelCase(): boolean { if (this.isEmpty()) { return false; } let zeroIndex: u64 = 0; let first: u32 = this.charCodeAt(zeroIndex); if (first < 97) { return false; } if (first > 122) { return false; } let index: u64 = 1; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"zunRuntime2\"; return value.isCamelCase(); }",
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
        "import { length, charCodeAt } from \"zun:core\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPascalCase(): boolean { if (this.isEmpty()) { return false; } let zeroIndex: u64 = 0; let first: u32 = this.charCodeAt(zeroIndex); if (first < 65) { return false; } if (first > 90) { return false; } let index: u64 = 1; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"ZunRuntime2\"; return value.isPascalCase(); }",
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
fn number_finiteness_and_safe_integer_predicates_are_equivalent() {
    assert_equivalent(
        "import { isFinite, isNaN } from \"zun:core\"; function main(): int { let value: number = 9007199254740991.0; if (!isFinite(value)) { return 0; } if (isNaN(value)) { return 0; } if ((value as int) as number != value) { return 0; } if (value < -9007199254740991.0) { return 0; } if (value > 9007199254740991.0) { return 0; } return 1; }",
    );
}

#[test]
fn number_negative_zero_predicate_is_jit_equivalent() {
    assert_equivalent(
        "import { isNegativeZero } from \"zun:core\"; function main(): boolean { return isNegativeZero(-0.0); }",
    );
}

#[test]
fn number_safe_integer_intrinsic_is_jit_equivalent() {
    assert_equivalent(
        "import { isSafeInteger } from \"zun:core\"; function main(): boolean { return isSafeInteger(9007199254740991.0); }",
    );
}

#[test]
fn math_power_is_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { powNumber } from \"zun:core\"; function main(): number { return powNumber(2.0, 3.0); }",
    )
    .expect("compile math power");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math");
    assert!(jit.is_compiled(entry));
    assert!(
        matches!(jit.call_entry(entry).expect("run JIT math"), RetVal::Float(value) if value == 8.0)
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 8.0));
}

#[test]
fn math_trigonometry_is_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { sinNumber, cosNumber } from \"zun:core\"; function main(): number { return sinNumber(0.0) + cosNumber(0.0); }",
    )
    .expect("compile math trigonometry");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math");
    assert!(jit.is_compiled(entry));
    assert!(
        matches!(jit.call_entry(entry).expect("run JIT math"), RetVal::Float(value) if value == 1.0)
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 1.0));
}

#[test]
fn math_exponential_and_logarithm_are_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { expNumber, logNumber } from \"zun:core\"; function main(): number { return expNumber(0.0) + logNumber(1.0); }",
    )
    .expect("compile math exponential and logarithm");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math");
    assert!(jit.is_compiled(entry));
    assert!(
        matches!(jit.call_entry(entry).expect("run JIT math"), RetVal::Float(value) if value == 1.0)
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 1.0));
}

#[test]
fn math_tangent_and_atan2_are_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { tanNumber, atan2Number } from \"zun:core\"; function main(): number { return tanNumber(0.0) + atan2Number(0.0, 1.0); }",
    )
    .expect("compile math tangent and atan2");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math");
    assert!(jit.is_compiled(entry));
    assert!(
        matches!(jit.call_entry(entry).expect("run JIT math"), RetVal::Float(value) if value == 0.0)
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 0.0));
}

#[test]
fn math_hypot_is_jit_equivalent() {
    let module = zun_compiler::compile(
        "import { hypotNumber } from \"zun:core\"; function main(): number { return hypotNumber(3.0, 4.0); }",
    )
    .expect("compile math hypot");
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with math");
    assert!(jit.is_compiled(entry));
    assert!(
        matches!(jit.call_entry(entry).expect("run JIT math"), RetVal::Float(value) if value == 5.0)
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 5.0));
}

#[test]
fn math_scalar_aliases_are_jit_equivalent() {
    let module = compile(
        "import { sqrtNumber, minNumber, maxNumber, floorNumber, ceilNumber, roundNumber } from \"zun:core\"; function main(): number { return sqrtNumber(9.0) + minNumber(2.0, 5.0) + maxNumber(2.0, 5.0) + floorNumber(2.9) + ceilNumber(2.1) + roundNumber(2.6); }",
    );
    let entry = module.entry_point.expect("entry point").rid();
    let jit = zun_codegen::Jit::compile(&module).expect("jit builds with scalar math");
    assert!(jit.is_compiled(entry));
    assert!(
        matches!(jit.call_entry(entry).expect("run JIT scalar math"), RetVal::Float(value) if value == 18.0)
    );
    assert!(matches!(interp_entry(&module), Value::Float(value, _) if value == 18.0));
}

#[test]
fn math_sign_preserves_zero_sign_in_jit() {
    assert_equivalent(
        "import { signNumber } from \"zun:core\"; function main(): number { return signNumber(-0.0); }",
    );
}

#[test]
fn math_trunc_is_jit_equivalent_for_both_signs() {
    assert_equivalent(
        "import { truncNumber } from \"zun:core\"; function main(): number { return truncNumber(-2.9) + truncNumber(2.9); }",
    );
}

#[test]
fn math_log_bases_are_jit_equivalent() {
    assert_equivalent(
        "import { log10Number, log2Number } from \"zun:core\"; function main(): number { return log10Number(100.0) + log2Number(8.0); }",
    );
}

#[test]
fn math_cbrt_is_jit_equivalent() {
    assert_equivalent(
        "import { cbrtNumber } from \"zun:core\"; function main(): number { return cbrtNumber(27.0); }",
    );
}

#[test]
fn math_expm1_and_log1p_are_jit_equivalent() {
    assert_equivalent("import { expm1Number, log1pNumber } from \"zun:core\"; function main(): number { return expm1Number(log1pNumber(2.0)); }");
}

#[test]
fn math_hyperbolic_functions_are_jit_equivalent() {
    assert_equivalent("import { sinhNumber, coshNumber, tanhNumber } from \"zun:core\"; function main(): number { return sinhNumber(1.0) + coshNumber(1.0) + tanhNumber(1.0); }");
}

#[test]
fn math_inverse_hyperbolic_functions_are_jit_equivalent() {
    assert_equivalent("import { asinhNumber, acoshNumber, atanhNumber } from \"zun:core\"; function main(): number { return asinhNumber(1.0) + acoshNumber(2.0) + atanhNumber(0.5); }");
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
    assert!(matches!(
        jit.call_entry(entry).expect("run JIT break"),
        RetVal::Int(3)
    ));
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
    assert!(matches!(
        jit.call_entry(entry).expect("run JIT continue"),
        RetVal::Int(12)
    ));
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
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Int(0, zun_il::NumKind::I32)
    ));
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
    assert!(
        !jit.is_compiled(entry),
        "Array instructions must remain Tier 0"
    );
    assert!(matches!(
        interp_entry(&module),
        Value::Float(9.0, zun_il::NumKind::F64)
    ));
}

