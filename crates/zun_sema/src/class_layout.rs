use std::collections::HashMap;
use std::rc::Rc;

use super::{Checker, FunctionCx};
use crate::hir;
use crate::hir::Ty;
use zun_syntax::ast;
use zun_common::Span;

impl<'a> Checker<'a> {
    /// The full field layout of a class: inherited fields first (base's full
    /// layout), then the class's own fields, in slot order.
    pub(super) fn full_fields(&self, class: u32) -> Vec<(String, Ty, bool, bool)> {
        (*self.full_fields_rc(class)).clone()
    }

    /// Shared, memoized backing for [`Self::full_fields`]. Each class's layout is
    /// built once (reusing its base's cached layout) and then reused, so the
    /// base chain is walked a constant number of times instead of per call.
    fn full_fields_rc(&self, class: u32) -> Rc<Vec<(String, Ty, bool, bool)>> {
        if let Some(cached) = self.full_fields_cache.borrow().get(&class) {
            return cached.clone();
        }
        let info = &self.classes[class as usize];
        let mut fields = match info.base {
            Some(b) => (*self.full_fields_rc(b)).clone(),
            None => Vec::new(),
        };
        // A field the class **redeclares** is the same property, not a second
        // one: `class B extends A { tag = "b" }` over `class A { tag = "a" }`
        // has one `tag`. Appending unconditionally gave it two slots, and a
        // read resolved to the inherited one, so `new B().tag` answered `"a"`
        // forever while the derived initializer wrote a slot nothing could see.
        // The derived declaration takes over the inherited slot; a static and an
        // instance field of one name stay distinct, as they are in JS.
        let inherited = fields.len();
        for field in &info.own_fields {
            match fields[..inherited]
                .iter()
                .position(|existing| existing.0 == field.0 && existing.2 == field.2)
            {
                Some(slot) => fields[slot] = field.clone(),
                None => fields.push(field.clone()),
            }
        }
        let rc = Rc::new(fields);
        self.full_fields_cache
            .borrow_mut()
            .insert(class, rc.clone());
        rc
    }

    /// Field initializers in the same inherited-first slot order as the layout.
    pub(super) fn full_initializers(&self, class: u32) -> Vec<Option<Box<zun_syntax::ast::Expr>>> {
        (*self.full_initializers_rc(class)).clone()
    }

    /// Shared, memoized backing for [`Self::full_initializers`].
    fn full_initializers_rc(&self, class: u32) -> Rc<Vec<Option<Box<zun_syntax::ast::Expr>>>> {
        if let Some(cached) = self.full_initializers_cache.borrow().get(&class) {
            return cached.clone();
        }
        let info = &self.classes[class as usize];
        let mut initializers = match info.base {
            Some(b) => (*self.full_initializers_rc(b)).clone(),
            None => Vec::new(),
        };
        // Index-aligned with `full_fields_rc`, so a redeclared field's
        // initializer has to land in the slot that layout reused -- the same
        // match, run over the inherited field names.
        let inherited_fields = match info.base {
            Some(b) => (*self.full_fields_rc(b)).clone(),
            None => Vec::new(),
        };
        let inherited = inherited_fields.len();
        for (index, initializer) in info.own_initializers.iter().enumerate() {
            let own = info.own_fields.get(index);
            let slot = own.and_then(|field| {
                inherited_fields
                    .iter()
                    .position(|existing| existing.0 == field.0 && existing.2 == field.2)
            });
            match slot.filter(|slot| *slot < inherited) {
                Some(slot) => initializers[slot] = initializer.clone(),
                None => initializers.push(initializer.clone()),
            }
        }
        let rc = Rc::new(initializers);
        self.full_initializers_cache
            .borrow_mut()
            .insert(class, rc.clone());
        rc
    }

    /// Memoized `field name -> (first slot, type)` index for `class`, matching
    /// the first-occurrence semantics of the previous linear `.find` scan.
    fn field_index(&self, class: u32) -> Rc<HashMap<String, (u16, Ty)>> {
        if let Some(cached) = self.field_index_cache.borrow().get(&class) {
            return cached.clone();
        }
        let mut map: HashMap<String, (u16, Ty)> = HashMap::new();
        for (slot, (name, ty, _, _)) in self.full_fields_rc(class).iter().enumerate() {
            map.entry(name.clone()).or_insert((slot as u16, *ty));
        }
        let rc = Rc::new(map);
        self.field_index_cache
            .borrow_mut()
            .insert(class, rc.clone());
        rc
    }

    /// Find a field's slot and type, searching inherited fields too.
    pub(super) fn field_of(&self, class: u32, name: &str) -> Option<(u16, Ty)> {
        self.field_index(class).get(name).copied()
    }

    pub(super) fn static_field_of(&self, class: u32, name: &str) -> Option<(u16, Ty)> {
        self.full_fields_rc(class)
            .iter()
            .enumerate()
            .find(|(_, (n, _, is_static, _))| n == name && *is_static)
            .map(|(slot, (_, ty, _, _))| (slot as u16, *ty))
    }

    /// True when `class` declares a private method or accessor named `name`
    /// (the name keeps its `#`), i.e. `#name` is not a slot but a member.
    pub(super) fn has_private_member(&self, class: u32, name: &str) -> bool {
        let Some(methods) = self.methods_by_class.get(class as usize) else {
            return false;
        };
        methods.contains_key(name)
            || methods.contains_key(&format!("get_{name}"))
            || methods.contains_key(&format!("set_{name}"))
    }

    pub(super) fn private_field_of(&self, class: u32, name: &str) -> Option<(u16, Ty)> {
        let owner = self
            .classes_by_name
            .iter()
            .find_map(|(name, &index)| (index == class).then_some(name.as_str()))?;
        let internal_name = format!("#{owner}#{name}");
        // Internal names (`#owner#field`) are always private, so a name hit in
        // the field index is necessarily the private field we are after.
        self.field_index(class).get(&internal_name).copied()
    }

    /// Resolve a method name to its function index, searching base classes.
    pub(super) fn method_of(&self, class: u32, name: &str) -> Option<u32> {
        if let Some(&f) = self.methods_by_class[class as usize].get(name) {
            return Some(f);
        }
        match self.classes[class as usize].base {
            Some(b) => self.method_of(b, name),
            None => None,
        }
    }

    /// Whether `sub` is `sup` or a (transitive) subclass of it.
    pub(super) fn is_subclass(&self, sub: u32, sup: u32) -> bool {
        if sub == sup {
            return true;
        }
        match self.classes[sub as usize].base {
            Some(b) => self.is_subclass(b, sup),
            None => false,
        }
    }

    /// Whether a value of type `from` may be used where `to` is expected.
    /// Instances are assignable to a base type; otherwise types must match.
    pub(super) fn is_assignable(&self, from: Ty, to: Ty) -> bool {
        match (from, to) {
            (Ty::Instance(d), Ty::Instance(b)) => self.is_subclass(d, b),
            (Ty::Function(_), Ty::Function(expected)) if expected == hir::DYNAMIC_FUNCTION => true,
            (Ty::Function(_), Ty::Function(expected)) if expected == hir::DYNAMIC_PREDICATE => true,
            (Ty::Function(_), Ty::Function(expected)) if expected == hir::DYNAMIC_REDUCER => true,
            (_, Ty::Any) => true,
            // `any` is assignable to any target (TS): a dynamic value flows into
            // a typed position and is coerced/unboxed at run time.
            (Ty::Any, _) => true,
            (a, b) => a == b,
        }
    }
}

impl<'a> Checker<'a> {
    /// The class that declares static field `name` as seen from `class`: the
    /// class itself when it declares (or shadows) it, else the nearest base
    /// that does. An inherited static is stored once, on its declaring class.
    pub(super) fn static_field_owner(&self, class: u32, name: &str) -> u32 {
        let mut current = class;
        loop {
            let info = &self.classes[current as usize];
            if info
                .own_fields
                .iter()
                .any(|(n, _, is_static, _)| n == name && *is_static)
            {
                return current;
            }
            match info.base {
                Some(base) => current = base,
                None => return class,
            }
        }
    }
}

/// What a `receiver.#name` expression does with the private member.
pub(super) enum PrivateUse<'a> {
    /// `receiver.#name`
    Read,
    /// `receiver.#name = value`
    Write(&'a ast::Expr),
    /// `receiver.#name(args)`
    Call(&'a [ast::Expr]),
}

impl Checker<'_> {
    /// Whether `class` itself declares the private name `name` (a field, a
    /// method or an accessor), and if so whether that member is static.
    pub(super) fn declares_private(&self, class: u32, name: &str) -> Option<bool> {
        let owner = self
            .classes_by_name
            .iter()
            .find_map(|(owner, &index)| (index == class).then_some(owner.as_str()))?;
        let internal = format!("#{owner}#{name}");
        if let Some((_, _, is_static, _)) = self.classes[class as usize]
            .own_fields
            .iter()
            .find(|(field, _, _, _)| *field == internal)
        {
            return Some(*is_static);
        }
        let methods = self.methods_by_class.get(class as usize)?;
        [
            name.to_string(),
            format!("get_{name}"),
            format!("set_{name}"),
        ]
        .iter()
        .find_map(|key| methods.get(key))
        .map(|&func| self.signatures[func as usize].is_static)
    }

    /// The class a private name used at byte offset `pos` resolves to, and
    /// whether the member is static. Private names are lexical: the innermost
    /// class whose body encloses the use and that declares the name wins, so a
    /// nested class sees the names of the classes around it.
    pub(super) fn private_owner_at(
        &self,
        this_class: Option<u32>,
        pos: u32,
        name: &str,
    ) -> Option<(u32, bool)> {
        for class in [this_class, self.cur_static_class].into_iter().flatten() {
            if let Some(is_static) = self.declares_private(class, name) {
                return Some((class, is_static));
            }
        }
        let mut enclosing: Vec<u32> = (0..self.classes.len() as u32)
            .filter(|&class| {
                let info = &self.classes[class as usize];
                info.span_start <= pos && pos < info.span_end
            })
            .collect();
        enclosing.sort_by_key(|&class| std::cmp::Reverse(self.classes[class as usize].span_start));
        enclosing.into_iter().find_map(|class| {
            self.declares_private(class, name)
                .map(|is_static| (class, is_static))
        })
    }

    /// `receiver.#name` where `#name` is a *static* private of `owner` and the
    /// receiver is not the class's own name (`this.#x` in a static method, a
    /// class held in a variable). A static private lives only on the class that
    /// declares it, so the receiver is compared with that class at run time and
    /// anything else - a subclass included - throws a TypeError before the slot
    /// is touched.
    pub(super) fn branded_static_private(
        &mut self,
        cx: &mut FunctionCx,
        owner: u32,
        receiver: &ast::Expr,
        field: &ast::Name,
        usage: PrivateUse<'_>,
        span: Span,
    ) -> Option<hir::Expr> {
        let owner_name = self
            .classes_by_name
            .iter()
            .find_map(|(name, &index)| (index == owner).then(|| name.clone()))?;
        let class_ident = ast::synthetic_ident(&owner_name);
        // The class name must still mean the class here, or the by-name access
        // below would not be the declaring class's slot.
        if cx.lookup(&owner_name).is_some()
            || self.classes_by_name.get(self.text(&class_ident)).copied() != Some(owner)
        {
            return None;
        }
        // A write evaluates the receiver, then the stored value, and only then
        // checks the brand, so both are computed into temporaries first. A
        // literal has no effect to order and keeps its literal typing.
        let is_literal = |expr: &ast::Expr| {
            matches!(
                expr,
                ast::Expr::Number { .. } | ast::Expr::String { .. } | ast::Expr::Bool { .. }
            )
        };
        let mut prelude: Vec<hir::Expr> = Vec::new();
        let mut receiver = receiver.clone();
        let mut value = match usage {
            PrivateUse::Write(value) => Some(value.clone()),
            _ => None,
        };
        if value.as_ref().is_some_and(|value| !is_literal(value)) {
            for operand in [Some(&mut receiver), value.as_mut()].into_iter().flatten() {
                let checked = self.check_expr(cx, operand);
                let ty = checked.ty;
                if ty.is_error() {
                    return Some(checked);
                }
                let (slot, tmp) = self.synthetic_local(cx, ty);
                prelude.push(hir::Expr {
                    kind: hir::ExprKind::Assign {
                        slot,
                        value: Box::new(checked),
                    },
                    ty,
                    span,
                });
                *operand = ast::Expr::Ident(tmp);
            }
        }
        let condition = self.check_condition(
            cx,
            &ast::Expr::Binary {
                op: ast::BinaryOp::Eq,
                lhs: Box::new(receiver),
                rhs: Box::new(ast::Expr::Ident(class_ident)),
                span,
            },
        );
        let by_name = ast::Expr::PrivateField {
            object: Box::new(ast::Expr::Ident(class_ident)),
            field: *field,
            span,
        };
        let access = match (usage, value) {
            (PrivateUse::Call(args), _) => ast::Expr::MethodCall {
                receiver: Box::new(ast::Expr::Ident(class_ident)),
                method: *field,
                args: args.to_vec(),
                span,
            },
            (_, Some(value)) => ast::Expr::FieldAssign {
                object: Box::new(by_name),
                field: *field,
                value: Box::new(value),
                span,
            },
            _ => by_name,
        };
        let when_true = self.check_expr(cx, &access);
        let ty = when_true.ty;
        let thrown = self.runtime_type_error(cx, span);
        let when_false = if ty == Ty::Any || ty.is_error() {
            thrown
        } else {
            hir::Expr {
                kind: hir::ExprKind::Convert {
                    value: Box::new(thrown),
                },
                ty,
                span,
            }
        };
        let mut result = hir::Expr {
            kind: hir::ExprKind::Conditional {
                condition: Box::new(condition),
                when_true: Box::new(when_true),
                when_false: Box::new(when_false),
            },
            ty,
            span,
        };
        for effect in prelude.into_iter().rev() {
            result = hir::Expr {
                kind: hir::ExprKind::Sequence {
                    effect: Box::new(effect),
                    value: Box::new(result),
                },
                ty,
                span,
            };
        }
        Some(result)
    }
}
