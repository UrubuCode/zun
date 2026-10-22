//! The mandatory format round-trip: build a module, serialize it, read it back,
//! and check that code (via disassembly), heaps, and tables all survive intact.

use zun_il::{disassemble, encode, Instruction, Opcode, Operand};
use zun_metadata::{FieldDef, MethodDef, TableKind, Token, TypeDef};
use zun_module::{Module, ReadError, FORMAT_MAJOR, MAGIC};

/// Build a small but representative module: one type with a field and a method
/// whose body does `local0 = 7 + 35; call <method>; ret`.
fn sample_module() -> Module {
    let mut m = Module::new();

    let type_name = m.strings.intern("Greeter");
    let ns = m.strings.intern("app");
    let field_name = m.strings.intern("count");
    let method_name = m.strings.intern("main");
    let field_sig = m.blobs.intern(&[0x01]); // placeholder signature blobs
    let method_sig = m.blobs.intern(&[0x02, 0x00]);

    let field_tok = m.tables.add_field_def(FieldDef {
        name: field_name,
        signature: field_sig,
        kind: 2, // i32
        is_static: true,
        is_private: true,
    });

    // The method body starts at code offset 0.
    let method_tok = m.tables.add_method_def(MethodDef {
        name: method_name,
        signature: method_sig,
        body_offset: 0,
        is_static: false,
    });

    m.tables.add_type_def(TypeDef {
        name: type_name,
        namespace: ns,
        base: Token::null(TableKind::TypeDef),
        field_start: field_tok.rid(),
        field_end: field_tok.rid() + 1,
        method_start: method_tok.rid(),
        method_end: method_tok.rid() + 1,
    });

    let body = [
        Instruction::new(Opcode::ConstI32, Operand::Imm32(7)),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(35)),
        Instruction::simple(Opcode::AddI32),
        Instruction::new(Opcode::StoreLocal, Operand::Local(0)),
        Instruction::new(Opcode::Call, Operand::Token(method_tok)),
        Instruction::simple(Opcode::Ret),
    ];
    for i in &body {
        encode(i, &mut m.code);
    }
    m.entry_point = Some(method_tok);
    m
}

#[test]
fn sequence_points_roundtrip_through_zm() {
    let mut module = Module::new();
    module.sequence_points.push(zun_module::SequencePoint {
        method_rid: 3,
        pc: 12,
        source_start: 40,
        source_end: 47,
    });
    let decoded = Module::read(&module.write()).expect("module with debug info");
    assert_eq!(decoded.sequence_points, module.sequence_points);
}

#[test]
fn emit_read_disassemble() {
    let original = sample_module();
    let bytes = original.write();

    let loaded = Module::read(&bytes).expect("module must read back");

    // Entry point and tables survive.
    assert_eq!(loaded.entry_point, original.entry_point);
    assert_eq!(loaded.tables.type_defs, original.tables.type_defs);
    assert_eq!(loaded.tables.field_defs, original.tables.field_defs);
    assert_eq!(loaded.tables.method_defs, original.tables.method_defs);

    // Heap content resolves through the loaded module.
    let ty = &loaded.tables.type_defs[0];
    assert_eq!(loaded.strings.get(ty.name), "Greeter");
    assert_eq!(loaded.strings.get(ty.namespace), "app");

    // Code survives byte-exactly and disassembles identically.
    assert_eq!(loaded.code, original.code);
    let text = disassemble(&loaded.code).unwrap();
    assert_eq!(
        text,
        "0000: const.i32 7\n\
         0005: const.i32 35\n\
         000A: add.i32\n\
         000B: stloc 0\n\
         000E: call MethodDef[1]\n\
         0013: ret\n"
    );
}

#[test]
fn write_is_deterministic() {
    assert_eq!(sample_module().write(), sample_module().write());
}

#[test]
fn empty_module_roundtrips() {
    let m = Module::new();
    let loaded = Module::read(&m.write()).unwrap();
    assert_eq!(loaded.entry_point, None);
    assert!(loaded.code.is_empty());
    assert!(loaded.tables.type_defs.is_empty());
}

#[test]
fn bad_magic_is_rejected() {
    let mut bytes = sample_module().write();
    bytes[0] = b'X';
    assert!(matches!(Module::read(&bytes), Err(ReadError::BadMagic)));
}

#[test]
fn newer_major_version_is_rejected() {
    let mut bytes = sample_module().write();
    let newer = (FORMAT_MAJOR + 1).to_le_bytes();
    bytes[4..6].copy_from_slice(&newer);
    match Module::read(&bytes) {
        Err(ReadError::UnsupportedVersion { major }) => assert_eq!(major, FORMAT_MAJOR + 1),
        other => panic!("expected version rejection, got {other:?}"),
    }
}

#[test]
fn truncated_file_is_rejected() {
    let bytes = sample_module().write();
    assert!(matches!(
        Module::read(&bytes[..20]),
        Err(ReadError::Truncated)
    ));
    // Also truncate inside a section rather than the header.
    assert!(matches!(
        Module::read(&bytes[..bytes.len() - 3]),
        Err(ReadError::Truncated)
    ));
}

#[test]
fn magic_spells_zunm() {
    assert_eq!(&MAGIC, b"ZUNM");
}
