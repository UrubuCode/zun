use crate::check::*;

impl Checker<'_> {
    /// The result type of a binary operation, diagnosing bad operand types.
    pub(in crate::check) fn binary_result(
        &mut self,
        op: ast::BinaryOp,
        lhs: &hir::Expr,
        rhs: &hir::Expr,
        span: Span,
    ) -> Ty {
        use ast::BinaryOp::*;
        if lhs.ty.is_error() || rhs.ty.is_error() {
            return Ty::Error;
        }
        // Both numeric operands must be the same numeric type (no implicit
        // int/float mixing; conversions are explicit — PLAN.md §4.2).
        // Both operands must be the same numeric type, exact width included.
        let same_numeric = |a: Ty, b: Ty| {
            matches!((a, b),
            (Ty::Int(x), Ty::Int(y)) if x == y)
                || matches!((a, b), (Ty::Float(x), Ty::Float(y)) if x == y)
        };
        match op {
            Nullish => {
                // `a ?? b` yields `b`'s type when `a` is dynamic; any other
                // operand pairing coerces through the dynamic path (JS).
                if lhs.ty == Ty::Any {
                    rhs.ty
                } else {
                    Ty::Any
                }
            }
            And | Or => {
                // Boolean operands keep a boolean result; otherwise the
                // operator follows JS truthiness over arbitrary values and the
                // result is dynamic.
                if lhs.ty == Ty::Boolean && rhs.ty == Ty::Boolean {
                    Ty::Boolean
                } else {
                    Ty::Any
                }
            }
            Add => {
                // `+` is numeric addition or string concatenation.
                if same_numeric(lhs.ty, rhs.ty) {
                    lhs.ty
                } else if lhs.ty == Ty::String && rhs.ty == Ty::String {
                    Ty::String
                } else {
                    self.error(span, format!("cannot add {:?} and {:?}", lhs.ty, rhs.ty));
                    Ty::Error
                }
            }
            Sub | Mul | Div | Rem => {
                if same_numeric(lhs.ty, rhs.ty) {
                    lhs.ty
                } else if lhs.ty == Ty::Any || rhs.ty == Ty::Any {
                    // A dynamic operand arithmetically coerces at run time (JS).
                    Ty::Any
                } else {
                    self.error(
                        span,
                        format!(
                            "arithmetic requires matching numbers, found {:?} and {:?}",
                            lhs.ty, rhs.ty
                        ),
                    );
                    Ty::Error
                }
            }
            // `**` on integers → `pow.i32`; `number ** number` is intercepted
            // and routed to `Math.pow` before reaching here, so a float operand
            // here is the unsupported `float32` case.
            Pow => {
                if lhs.ty.is_int() && same_numeric(lhs.ty, rhs.ty) {
                    lhs.ty
                } else {
                    self.error(
                        span,
                        format!(
                            "exponentiation requires matching int or number operands, found {:?} and {:?}",
                            lhs.ty, rhs.ty
                        ),
                    );
                    Ty::Error
                }
            }
            // Bitwise and shift operators. On integer operands they use the
            // native width/signedness (`shr` is arithmetic for signed, logical
            // for unsigned). On `number` (f64) they follow JS: coerce both
            // operands to int32 (uint32 for the `>>>` left operand), operate,
            // and widen the int result back to a `number` — all lowered in
            // ilgen, so the result type here is the operand's own numeric type.
            BitAnd | BitOr | BitXor | Shl | Shr | UShr => {
                let both_int = lhs.ty.is_int() && same_numeric(lhs.ty, rhs.ty);
                let both_number = lhs.ty == Ty::F64 && rhs.ty == Ty::F64;
                if both_int || both_number {
                    lhs.ty
                } else if lhs.ty == Ty::Any || rhs.ty == Ty::Any {
                    // A dynamic operand is coerced to int32/uint32 at run time (JS).
                    Ty::Any
                } else {
                    self.error(
                        span,
                        format!(
                            "bitwise/shift operators require matching int or number operands, found {:?} and {:?}",
                            lhs.ty, rhs.ty
                        ),
                    );
                    Ty::Error
                }
            }
            Lt | Le | Gt | Ge => {
                // A dynamic (`any`) operand compares at run time (JS relational
                // coercion); otherwise both sides must be the same number type.
                if same_numeric(lhs.ty, rhs.ty)
                    || lhs.ty == Ty::Any
                    || rhs.ty == Ty::Any
                    || (lhs.ty == Ty::String && rhs.ty == Ty::String)
                    // Mixed primitives compare numerically at run time (JS).
                    || (matches!(lhs.ty, Ty::String | Ty::BigInt | Ty::Boolean | Ty::Void) && rhs.ty == Ty::F64)
                    || (lhs.ty == Ty::F64 && matches!(rhs.ty, Ty::String | Ty::BigInt | Ty::Boolean | Ty::Void))
                {
                    Ty::Boolean
                } else {
                    self.error(
                        span,
                        format!(
                            "comparison requires matching numbers, found {:?} and {:?}",
                            lhs.ty, rhs.ty
                        ),
                    );
                    Ty::Error
                }
            }
            Eq | Ne | LooseEq | LooseNe => {
                // Any two function values are comparable by reference identity,
                // even with different signatures (distinct `Function(_)` types):
                // different functions are simply unequal. A dynamic (`any`)
                // operand makes the comparison run-time (JS allows comparing any
                // two values), so it is accepted against anything.
                let both_functions =
                    matches!(lhs.ty, Ty::Function(_)) && matches!(rhs.ty, Ty::Function(_));
                if lhs.ty == rhs.ty
                    || both_functions
                    || lhs.ty == Ty::Any
                    || rhs.ty == Ty::Any
                {
                    Ty::Boolean
                } else {
                    self.error(
                        span,
                        format!("cannot compare {:?} with {:?}", lhs.ty, rhs.ty),
                    );
                    Ty::Error
                }
            }
            // `in`/`instanceof` are intercepted and lowered before reaching
            // here; this arm only satisfies exhaustiveness.
            In | Instanceof => Ty::Boolean,
        }
    }
}
