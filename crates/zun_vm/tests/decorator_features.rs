//! Typed `DecoratorContext` features for runtime TC39 decorators:
//! `context.addInitializer` (class decorators), and field decorators running
//! at class-definition time.

use zun_interp::CaptureOutput;

fn run(user: &str) -> Vec<String> {
    let module = zun_compiler::compile(user).expect("program should compile");
    let bytes = module.write();
    let mut out = CaptureOutput::default();
    zun_vm::execute_with(&bytes, &mut out).expect("program should run");
    out.lines
}

#[test]
fn typed_context_exposes_kind_and_name() {
    let out = run(
        "function tag(value: any, context: DecoratorContext): void {\n\
            print(context.kind);\n\
            print(context.name);\n\
         }\n\
         @tag\n\
         class Widget { id: number; }\n\
         function main() { print(new Widget(1.0).id); }",
    );
    assert_eq!(out, ["class", "Widget", "1"]);
}

#[test]
fn class_decorator_add_initializer_runs_after_definition() {
    let out = run(
        "function boot(value: any, context: DecoratorContext): void {\n\
            print(\"decorating\");\n\
            context.addInitializer((target: any) => { print(\"init-1\"); });\n\
            context.addInitializer((target: any) => { print(\"init-2\"); });\n\
            print(\"decorated\");\n\
         }\n\
         @boot\n\
         class Service { id: number; }\n\
         function main() { print(\"main\"); }",
    );
    assert_eq!(out, ["decorating", "decorated", "init-1", "init-2", "main"]);
}

#[test]
fn field_decorator_runs_with_kind_field() {
    let out = run(
        "function watch(value: any, context: DecoratorContext): void {\n\
            print(context.kind);\n\
            print(context.name);\n\
         }\n\
         class Box { @watch size: number; }\n\
         function main() { print(new Box(3.0).size); }",
    );
    assert_eq!(out, ["field", "size", "3"]);
}
