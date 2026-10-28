//! Metadata lookup helpers used by interpreter execution.

use super::{Interp, RunError};
use super::operands::operand_strref;
use zun_il::Operand;

impl<'a> Interp<'a> {
    pub(crate) fn type_field_range(
        &self,
        token: zun_metadata::Token,
    ) -> Result<(u32, usize), RunError> {
        let ty = self
            .module
            .tables
            .type_defs
            .get(token.rid() as usize - 1)
            .ok_or(RunError::BadToken)?;
        Ok((ty.field_start, (ty.field_end - ty.field_start) as usize))
    }

    pub(crate) fn field_kind_at(&self, field_rid: u32) -> Result<u8, RunError> {
        self.module
            .tables
            .field_defs
            .get(field_rid as usize - 1)
            .map(|field| field.kind)
            .ok_or(RunError::BadToken)
    }

    pub(crate) fn static_field_key(
        &self,
        token: zun_metadata::Token,
    ) -> Result<(u32, u32), RunError> {
        if token.table() != Some(zun_metadata::TableKind::FieldDef) || token.rid() == 0 {
            return Err(RunError::BadToken);
        }
        let field = self.module.tables.field_defs.get(token.rid() as usize - 1).ok_or(RunError::BadToken)?;
        if !field.is_static {
            return Err(RunError::BadToken);
        }
        let owner = self.module.tables.type_defs.iter().enumerate()
            .find(|(_, ty)| ty.field_start <= token.rid() && token.rid() < ty.field_end)
            .map(|(index, _)| index as u32 + 1)
            .ok_or(RunError::BadToken)?;
        Ok((owner, token.rid()))
    }

    /// The declared value-type code of a static field, used to restore the exact
    /// numeric width when loading it back from the width-erasing `Any` store.
    pub(crate) fn static_field_kind(
        &self,
        token: zun_metadata::Token,
    ) -> Result<u8, RunError> {
        if token.table() != Some(zun_metadata::TableKind::FieldDef) || token.rid() == 0 {
            return Err(RunError::BadToken);
        }
        self.module
            .tables
            .field_defs
            .get(token.rid() as usize - 1)
            .map(|field| field.kind)
            .ok_or(RunError::BadToken)
    }

    pub(crate) fn dynamic_name(&self, operand: &Operand) -> Result<&str, RunError> {
        let index = operand_strref(operand);
        Ok(self.module.strings.get(zun_metadata::StringIndex(index)))
    }
}
