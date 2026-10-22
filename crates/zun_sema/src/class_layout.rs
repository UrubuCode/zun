use super::Checker;
use crate::hir;
use crate::hir::Ty;

impl<'a> Checker<'a> {
    /// The full field layout of a class: inherited fields first (base's full
    /// layout), then the class's own fields, in slot order.
    pub(super) fn full_fields(&self, class: u32) -> Vec<(String, Ty, bool, bool)> {
        let info = &self.classes[class as usize];
        let mut fields = match info.base {
            Some(b) => self.full_fields(b),
            None => Vec::new(),
        };
        fields.extend(info.own_fields.iter().cloned());
        fields
    }

    /// Find a field's slot and type, searching inherited fields too.
    pub(super) fn field_of(&self, class: u32, name: &str) -> Option<(u16, Ty)> {
        self.full_fields(class)
            .into_iter()
            .enumerate()
            .find(|(_, (n, _, _, _))| n == name)
            .map(|(slot, (_, ty, _, _))| (slot as u16, ty))
    }

    pub(super) fn static_field_of(&self, class: u32, name: &str) -> Option<(u16, Ty)> {
        self.full_fields(class)
            .into_iter()
            .enumerate()
            .find(|(_, (n, _, is_static, _))| n == name && *is_static)
            .map(|(slot, (_, ty, _, _))| (slot as u16, ty))
    }

    pub(super) fn private_field_of(&self, class: u32, name: &str) -> Option<(u16, Ty)> {
        let owner = self
            .classes_by_name
            .iter()
            .find_map(|(name, &index)| (index == class).then_some(name.as_str()))?;
        let internal_name = format!("#{owner}#{name}");
        self.full_fields(class)
            .into_iter()
            .enumerate()
            .find(|(_, (n, _, _, is_private))| *is_private && n == &internal_name)
            .map(|(slot, (_, ty, _, _))| (slot as u16, ty))
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
            (a, b) => a == b,
        }
    }
}
