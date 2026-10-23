//! `import.dynamic` is async + dynamic, so it must stay on Tier 0: the typed
//! JIT never lists it, and a method containing it is not JIT-eligible.
//! (The operand is irrelevant to eligibility, so a plain constant stands in.)

use zun_codegen::eligible_methods_with_literals;
use zun_il::{encode, Instruction, JitKind, NumKind, Opcode, Operand};
use zun_metadata::{MethodDef, TableKind};
use zun_module::{write_body, Module};

fn module_with(body: &[Instruction]) -> Module {
    let mut module = Module::new();
    let mut code = Vec::new();
    for instruction in body {
        encode(instruction, &mut code);
    }
    let offset = write_body(
        &mut module.code,
        0,
        0,
        true,
        JitKind::Num(NumKind::I32).to_byte(),
        &[],
        &code,
    );
    module.tables.add_method_def(MethodDef {
        name: module.strings.intern("main"),
        signature: zun_metadata::BlobIndex(0),
        body_offset: offset,
        is_static: false,
    });
    module.entry_point = Some(zun_metadata::Token::new(TableKind::MethodDef, 1));
    module
}

#[test]
fn import_dynamic_is_not_jit_eligible() {
    let control = module_with(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::simple(Opcode::Pop),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(1)),
        Instruction::simple(Opcode::Ret),
    ]);
    assert!(eligible_methods_with_literals(&control, false).contains(&1));

    let dynamic = module_with(&[
        Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
        Instruction::simple(Opcode::ImportDynamic),
        Instruction::simple(Opcode::Pop),
        Instruction::new(Opcode::ConstI32, Operand::Imm32(1)),
        Instruction::simple(Opcode::Ret),
    ]);
    assert!(!eligible_methods_with_literals(&dynamic, false).contains(&1));
}
