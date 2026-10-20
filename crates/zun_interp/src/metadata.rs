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

    pub(crate) fn dynamic_name(&self, operand: &Operand) -> Result<&str, RunError> {
        let index = operand_strref(operand);
        Ok(self.module.strings.get(zun_metadata::StringIndex(index)))
    }
}
