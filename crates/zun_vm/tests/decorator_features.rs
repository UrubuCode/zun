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

#[test]
fn method_decorator_return_value_replaces_the_method() {
    // The returned function receives the instance as its leading argument and
    // replaces the method for every call site.
    let out = run(
        "function double(value: any, context: DecoratorContext): Function {\n\
            return (self: any, n: number) => n * 2.0;\n\
         }\n\
         class Calc {\n\
            base: number;\n\
            @double add(n: number): number { return n + 1.0; }\n\
            plain(n: number): number { return n + 1.0; }\n\
         }\n\
         function main() {\n\
            let c = new Calc(0.0);\n\
            print(c.add(5.0));\n\
            print(c.plain(5.0));\n\
         }",
    );
    assert_eq!(out, ["10", "6"]);
}

#[test]
fn method_decorator_wrapper_can_call_the_original() {
    let out = run(
        "function twice(value: any, context: DecoratorContext): Function {\n\
            return (self: any, n: number) => (Reflect.apply(value, self, [n])) as number * 2.0;\n\
         }\n\
         class Calc {\n\
            base: number;\n\
            @twice add(n: number): number { return n + this.base; }\n\
         }\n\
         function main() {\n\
            let c = new Calc(100.0);\n\
            print(c.add(5.0));\n\
         }",
    );
    assert_eq!(out, ["210"]);
}

#[test]
fn replaced_method_dispatches_to_subclass_overrides() {
    let out = run(
        "function loud(value: any, context: DecoratorContext): Function {\n\
            return (self: any, n: number) => n * 100.0;\n\
         }\n\
         class Base {\n\
            tag: number;\n\
            @loud run(n: number): number { return n; }\n\
         }\n\
         class Child extends Base {\n\
            run(n: number): number { return n + 1.0; }\n\
         }\n\
         function call(b: Base): number { return b.run(2.0); }\n\
         function main() {\n\
            print(call(new Base(0.0)));\n\
            print(call(new Child(0.0)));\n\
         }",
    );
    assert_eq!(out, ["200", "3"]);
}

#[test]
fn instance_add_initializer_runs_with_this_per_instance() {
    let out = run(
        "function track(value: any, context: DecoratorContext): void {\n\
            context.addInitializer((self: Counter) => { print(self.count); });\n\
         }\n\
         class Counter { @track count: number; }\n\
         function main() {\n\
            print(\"start\");\n\
            new Counter(7.0);\n\
            new Counter(8.0);\n\
         }",
    );
    assert_eq!(out, ["start", "7", "8"]);
}

#[test]
fn method_decorator_instance_initializer_runs_on_construction() {
    let out = run(
        "function boot(value: any, context: DecoratorContext): void {\n\
            context.addInitializer((self: Service) => { print(self.id); });\n\
         }\n\
         class Service {\n\
            id: number;\n\
            @boot start(): void { print(\"started\"); }\n\
         }\n\
         function main() {\n\
            let s = new Service(3.0);\n\
            s.start();\n\
         }",
    );
    assert_eq!(out, ["3", "started"]);
}

#[test]
fn field_decorator_return_value_transforms_the_initial_value() {
    let out = run(
        "function plusTen(value: any, context: DecoratorContext): Function {\n\
            return (initial: number) => initial + 10.0;\n\
         }\n\
         class Box { @plusTen size: number; other: number; }\n\
         function main() {\n\
            let b = new Box(3.0, 4.0);\n\
            print(b.size);\n\
            print(b.other);\n\
         }",
    );
    assert_eq!(out, ["13", "4"]);
}

#[test]
fn context_access_reads_and_writes_through_an_instance() {
    let out = run(
        "function expose(value: any, context: DecoratorContext): void {\n\
            context.addInitializer((self: Gauge) => {\n\
                print((context.access.get(self)) as number);\n\
                context.access.set(self, 99.0);\n\
            });\n\
         }\n\
         class Gauge { @expose level: number; }\n\
         function main() {\n\
            let g = new Gauge(5.0);\n\
            print(g.level);\n\
         }",
    );
    assert_eq!(out, ["5", "99"]);
}

#[test]
fn queued_class_initializers_survive_collections_between_decorator_and_drain() {
    // The decorator allocates enough to trigger collections after queuing a
    // capturing closure; the closure environment must still be alive at drain.
    let out = run(
        "function churn(value: any, context: DecoratorContext): void {\n\
            let label = \"kept\";\n\
            context.addInitializer((target: any) => { print(label); });\n\
            let i = 0.0;\n\
            while (i < 20000.0) { let junk = [i, i, i]; i = i + 1.0; }\n\
         }\n\
         @churn\n\
         class Holder { id: number; }\n\
         function main() { print(\"main\"); }",
    );
    assert_eq!(out, ["kept", "main"]);
}
