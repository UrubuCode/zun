//! Small instruction-selection helpers extracted from IL generation.

use zun_il::Opcode;

/// Choose between the integer and float opcode for a numeric operation.
pub(crate) fn int_or_float(is_int: bool, int_op: Opcode, float_op: Opcode) -> Opcode {
    if is_int {
        int_op
    } else {
        float_op
    }
}
