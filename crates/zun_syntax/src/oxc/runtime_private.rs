//! Private names (`#x`) of a run-time class.
//!
//! A static class keeps its private members in the typed layout and brands
//! by type. A run-time class (see [`super::runtime_class`]) has neither, and
//! needs neither: a private name is per-evaluation state, exactly like a
//! captured local, so it lowers to bindings of the class factory.
//!
//! * an instance field is a `WeakMap` from instance to value;
//! * instance methods and accessors share one brand `WeakMap` and live, as
//!   closures, in a functions bag;
//! * static members are branded by identity with the class; static fields
//!   live in a statics bag.
//!
//! Nothing is stored on the instance, the class or the prototype, so no
//! reflection surface can list a private member. Every reference converts to
//! a call of a prelude helper (`__zunPriv*` in `builtin/runtime_class.zun`),
//! which performs the brand check and raises the `TypeError` a wrong
//! receiver gets.
//!
//! Private names are lexical. The frames below mirror the class nesting of
//! the node being converted, innermost last: a run-time class pushes its
//! slots, a static class pushes its names as *shadows* so that a reference
//! inside it is left to the checker.

use super::*;

/// What a private name of a run-time class is, and where it lives. `kind`
/// is the helper protocol's number (see the prelude): bit 0 is "static".
#[derive(Clone)]
pub(super) struct PrivateSlot {
    kind: u8,
    /// The brand: a `WeakMap` binding, or the class binding (static).
    guard: Name,
    /// The field's `WeakMap`, the statics bag, or the functions bag.
    store: Name,
    /// The name without its `#`.
    key: String,
    /// The class binding, for the receiver errors.
    class: Name,
}

type Frame = Vec<(String, Option<PrivateSlot>)>;

thread_local! {
    static PRIVATE_FRAMES: std::cell::RefCell<Vec<Frame>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Pops the frame pushed by [`enter_private_frame`] / [`shadow_private_names`].
pub(super) struct PrivateFrameGuard;

impl Drop for PrivateFrameGuard {
    fn drop(&mut self) {
        PRIVATE_FRAMES.with(|frames| {
            frames.borrow_mut().pop();
        });
    }
}

fn push_frame(frame: Frame) -> PrivateFrameGuard {
    PRIVATE_FRAMES.with(|frames| frames.borrow_mut().push(frame));
    PrivateFrameGuard
}

/// Enter a static class declaring `names`: inside it they belong to the
/// checker, even when an enclosing run-time class declares the same name.
pub(super) fn shadow_private_names<'n>(names: impl Iterator<Item = &'n str>) -> PrivateFrameGuard {
    push_frame(names.map(|name| (name.to_string(), None)).collect())
}

/// The run-time class slot `#name` resolves to at the node being converted,
/// if its innermost declaring class is a run-time class.
pub(super) fn runtime_private(name: &str) -> Option<PrivateSlot> {
    PRIVATE_FRAMES.with(|frames| {
        frames
            .borrow()
            .iter()
            .rev()
            .find_map(|frame| frame.iter().find(|(declared, _)| declared == name))
            .and_then(|(_, slot)| slot.clone())
    })
}

fn ident(name: Name) -> Expr {
    Expr::Ident(name)
}

fn helper(name: &str, args: Vec<Expr>, span: Span) -> Expr {
    Expr::Call {
        callee: crate::ast::synthetic_ident(name),
        args,
        span,
    }
}

fn text(value: &str, span: Span) -> Expr {
    Expr::String {
        value: value.to_string(),
        span,
    }
}

impl PrivateSlot {
    /// The arguments every reference helper starts with.
    fn lead(&self, object: Expr, span: Span) -> Vec<Expr> {
        vec![
            Expr::Number {
                value: f64::from(self.kind),
                span,
            },
            ident(self.guard),
            object,
            ident(self.store),
            text(&self.key, span),
            ident(self.class),
        ]
    }

    /// `object.#name`
    pub(super) fn read(&self, object: Expr, span: Span) -> Expr {
        helper("__zunPrivRead", self.lead(object, span), span)
    }

    /// `object.#name = value`
    pub(super) fn write(&self, object: Expr, value: Expr, span: Span) -> Expr {
        let mut args = self.lead(object, span);
        args.push(value);
        helper("__zunPrivWrite", args, span)
    }

    /// `object.#name(args)`
    pub(super) fn call(&self, object: Expr, args: Vec<Expr>, span: Span) -> Expr {
        let elements = args
            .into_iter()
            .map(|arg| match arg {
                Expr::Spread { expr, .. } => crate::ast::ArrayElement::Spread(*expr),
                other => crate::ast::ArrayElement::Expr(other),
            })
            .collect();
        let mut lead = self.lead(object, span);
        lead.push(Expr::ArrayLiteral { elements, span });
        helper("__zunPrivCall", lead, span)
    }

    /// `#name in object`
    pub(super) fn has(&self, object: Expr, span: Span) -> Expr {
        helper(
            "__zunPrivIn",
            vec![
                Expr::Number {
                    value: f64::from(self.kind),
                    span,
                },
                ident(self.guard),
                object,
                text(&self.key, span),
            ],
            span,
        )
    }
}

/// The private members of one run-time class: the bindings that hold them
/// and the statements that create those bindings.
pub(super) struct PrivateMembers {
    frame: Frame,
    /// `let` statements for the stores, to run before any member closure is
    /// created (a closure only captures a binding that already exists).
    pub(super) declarations: Vec<Stmt>,
    functions: Name,
    statics: Name,
    brand: Option<Name>,
}

impl PrivateMembers {
    /// Collect the private names `class` declares. `unique` distinguishes the
    /// bindings of this class from those of any other.
    pub(super) fn collect(
        class: &oxc_ast::ast::Class<'_>,
        class_binding: Name,
        unique: u32,
        whole: Span,
    ) -> Self {
        let functions = crate::ast::synthetic_ident(&format!("__zcPF{unique}"));
        let statics = crate::ast::synthetic_ident(&format!("__zcPS{unique}"));
        let brand_binding = crate::ast::synthetic_ident(&format!("__zcPB{unique}"));
        let any = || {
            Some(crate::ast::TypeAnnotation {
                name: crate::ast::synthetic_ident("any"),
                is_array: false,
                array_depth: 0,
                nullable: false,
            })
        };
        let store = |name: Name| Stmt::Let {
            name,
            ty: any(),
            init: helper("__zunPrivStore", Vec::new(), whole),
            span: whole,
        };
        let bag = |name: Name| Stmt::Let {
            name,
            ty: any(),
            init: Expr::ObjectLiteral {
                properties: Vec::new(),
                span: whole,
            },
            span: whole,
        };
        let mut frame: Frame = Vec::new();
        let mut declarations = Vec::new();
        let mut brand = None;
        let (mut any_function, mut any_static) = (false, false);
        for (index, element) in class.body.body.iter().enumerate() {
            let (name, is_static, member) = match element {
                ClassElement::PropertyDefinition(property) => match &property.key {
                    PropertyKey::PrivateIdentifier(identifier) => {
                        (identifier.name.as_str(), property.r#static, 0_u8)
                    }
                    _ => continue,
                },
                ClassElement::MethodDefinition(method) => match &method.key {
                    PropertyKey::PrivateIdentifier(identifier) => (
                        identifier.name.as_str(),
                        method.r#static,
                        match method.kind {
                            MethodDefinitionKind::Get | MethodDefinitionKind::Set => 4,
                            _ => 2,
                        },
                    ),
                    _ => continue,
                },
                _ => continue,
            };
            // A getter and a setter share one name.
            if frame.iter().any(|(declared, _)| declared == name) {
                continue;
            }
            let (guard, store_binding) = match (member, is_static) {
                (0, false) => {
                    let map = crate::ast::synthetic_ident(&format!("__zcPW{unique}_{index}"));
                    declarations.push(store(map));
                    (map, map)
                }
                (0, true) => {
                    any_static = true;
                    (class_binding, statics)
                }
                (_, false) => {
                    any_function = true;
                    brand = Some(brand_binding);
                    (brand_binding, functions)
                }
                (_, true) => {
                    any_function = true;
                    (class_binding, functions)
                }
            };
            frame.push((
                name.to_string(),
                Some(PrivateSlot {
                    kind: member + u8::from(is_static),
                    guard,
                    store: store_binding,
                    key: name.to_string(),
                    class: class_binding,
                }),
            ));
        }
        if brand.is_some() {
            declarations.push(store(brand_binding));
        }
        if any_function {
            declarations.push(bag(functions));
        }
        if any_static {
            declarations.push(bag(statics));
        }
        Self {
            frame,
            declarations,
            functions,
            statics,
            brand,
        }
    }

    /// Make the names visible to the members about to be converted.
    pub(super) fn enter(&self) -> PrivateFrameGuard {
        push_frame(self.frame.clone())
    }

    /// The statement that brands a new instance for the private methods and
    /// accessors, to run before its fields are defined.
    pub(super) fn brand_instance(&self, this: Expr, span: Span) -> Option<Stmt> {
        self.brand.map(|brand| {
            Stmt::Expr(helper(
                "__zunPrivDefine",
                vec![
                    ident(brand),
                    this,
                    Expr::Bool { value: true, span },
                    text("", span),
                ],
                span,
            ))
        })
    }

    /// Define the instance private field `#name` on `this`.
    pub(super) fn define_field(&self, name: &str, this: Expr, value: Expr, span: Span) -> Stmt {
        let slot = self.slot(name);
        Stmt::Expr(helper(
            "__zunPrivDefine",
            vec![ident(slot.store), this, value, text(name, span)],
            span,
        ))
    }

    /// Initialize the static private field `#name`.
    pub(super) fn define_static(&self, name: &str, value: Expr, span: Span) -> Stmt {
        Stmt::Expr(Expr::IndexAssign {
            object: Box::new(ident(self.statics)),
            index: Box::new(text(name, span)),
            value: Box::new(value),
            span,
        })
    }

    /// Install the private method, getter or setter `#name`.
    pub(super) fn define_function(
        &self,
        name: &str,
        kind: MethodDefinitionKind,
        function: Expr,
        span: Span,
    ) -> Stmt {
        let key = match kind {
            MethodDefinitionKind::Get => format!("g:{name}"),
            MethodDefinitionKind::Set => format!("s:{name}"),
            _ => name.to_string(),
        };
        Stmt::Expr(Expr::IndexAssign {
            object: Box::new(ident(self.functions)),
            index: Box::new(text(&key, span)),
            value: Box::new(function),
            span,
        })
    }

    fn slot(&self, name: &str) -> &PrivateSlot {
        self.frame
            .iter()
            .find(|(declared, _)| declared == name)
            .and_then(|(_, slot)| slot.as_ref())
            .expect("a private member was collected before it is defined")
    }
}
