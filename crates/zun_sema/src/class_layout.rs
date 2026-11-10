use std::collections::HashMap;
use std::rc::Rc;

use super::Checker;
use crate::hir;
use crate::hir::Ty;

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
        fields.extend(info.own_fields.iter().cloned());
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
        initializers.extend(info.own_initializers.iter().cloned());
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
