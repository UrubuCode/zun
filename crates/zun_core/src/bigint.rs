//! Arbitrary-precision integer storage for the future `bigint` language type.
//!
//! The representation is a normalized sign-magnitude value with base-2^32
//! limbs in little-endian order. Keeping this floor independent of the
//! interpreter lets the compiler/runtime add a managed carrier later without
//! putting arithmetic or allocation policy in the TypeScript wrapper.

use core::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BigInt {
    negative: bool,
    limbs: Vec<u32>,
}

impl BigInt {
    pub fn zero() -> Self {
        Self {
            negative: false,
            limbs: Vec::new(),
        }
    }

    pub fn from_u64(value: u64) -> Self {
        let mut limbs = vec![value as u32];
        if value >> 32 != 0 {
            limbs.push((value >> 32) as u32);
        }
        Self {
            negative: false,
            limbs,
        }
        .normalized()
    }

    pub fn from_i64(value: i64) -> Self {
        if value < 0 {
            let magnitude = (-(value as i128)) as u128;
            Self::from_u128(magnitude, true)
        } else {
            Self::from_u128(value as u128, false)
        }
    }

    /// Parse a BigInt from text, honoring an optional sign and the `0x`/`0o`/
    /// `0b` radix prefixes (as accepted by `BigInt(string)` and `1n`-style
    /// literals). A bare body is decimal.
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        let trimmed = text.trim();
        let bytes = trimmed.as_bytes();
        if bytes.is_empty() {
            return Err("empty bigint literal");
        }
        let (negative, rest) = match bytes[0] {
            b'-' => (true, &trimmed[1..]),
            b'+' => (false, &trimmed[1..]),
            _ => (false, trimmed),
        };
        let (radix, digits) = if rest.len() >= 2 && &rest[..1] == "0" {
            match rest.as_bytes()[1] {
                b'x' | b'X' => (16u32, &rest[2..]),
                b'o' | b'O' => (8, &rest[2..]),
                b'b' | b'B' => (2, &rest[2..]),
                _ => (10, rest),
            }
        } else {
            (10, rest)
        };
        if digits.is_empty() {
            return Err("invalid bigint literal");
        }
        let mut result = Self::zero();
        for &byte in digits.as_bytes() {
            let value = match byte {
                b'0'..=b'9' => u32::from(byte - b'0'),
                b'a'..=b'f' => u32::from(byte - b'a') + 10,
                b'A'..=b'F' => u32::from(byte - b'A') + 10,
                b'_' => continue,
                _ => return Err("invalid bigint digit"),
            };
            if value >= radix {
                return Err("bigint digit out of range for radix");
            }
            result.mul_small(radix);
            result.add_small(value);
        }
        result.negative = negative && !result.is_zero();
        Ok(result)
    }

    /// Build a BigInt from an integer-valued `number`. A non-integer or
    /// non-finite input is rejected (matching JS `BigInt(1.5)` → RangeError).
    pub fn from_f64(value: f64) -> Result<Self, &'static str> {
        if !value.is_finite() {
            return Err("cannot convert a non-finite number to a BigInt");
        }
        if value == 0.0 {
            return Ok(Self::zero());
        }
        if value.fract() != 0.0 {
            return Err("cannot convert a non-integer number to a BigInt");
        }
        let negative = value.is_sign_negative();
        let bits = value.abs().to_bits();
        let raw_exp = ((bits >> 52) & 0x7ff) as i64;
        let fraction = bits & ((1u64 << 52) - 1);
        // Integer-valued finite f64 is always normal here (|value| >= 1).
        let mantissa = fraction | (1u64 << 52);
        let exponent = raw_exp - 1075; // 1023 bias + 52 fraction bits.
        let mut result = Self::from_u64(mantissa);
        if exponent > 0 {
            result = result.shl(exponent as u64);
        } else if exponent < 0 {
            // The value is integer-valued, so the dropped low bits are zero and
            // this right shift on the positive magnitude is exact.
            result = result.shr((-exponent) as u64);
        }
        result.negative = negative && !result.is_zero();
        Ok(result)
    }

    pub fn parse_decimal(text: &str) -> Result<Self, &'static str> {
        let bytes = text.as_bytes();
        if bytes.is_empty() {
            return Err("empty bigint literal");
        }
        let (negative, digits) = match bytes[0] {
            b'-' => (true, &bytes[1..]),
            b'+' => (false, &bytes[1..]),
            _ => (false, bytes),
        };
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return Err("invalid bigint decimal literal");
        }
        let mut result = Self::zero();
        for digit in digits {
            result.mul_small(10);
            result.add_small(u32::from(digit - b'0'));
        }
        result.negative = negative && !result.is_zero();
        Ok(result)
    }

    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }
    pub fn is_negative(&self) -> bool {
        self.negative
    }
    pub fn add(&self, other: &Self) -> Self {
        match (self.negative, other.negative) {
            (a, b) if a == b => Self {
                negative: a,
                limbs: add_abs(&self.limbs, &other.limbs),
            }
            .normalized(),
            _ => match cmp_abs(&self.limbs, &other.limbs) {
                Ordering::Equal => Self::zero(),
                Ordering::Greater => Self {
                    negative: self.negative,
                    limbs: sub_abs(&self.limbs, &other.limbs),
                }
                .normalized(),
                Ordering::Less => Self {
                    negative: other.negative,
                    limbs: sub_abs(&other.limbs, &self.limbs),
                }
                .normalized(),
            },
        }
    }

    pub fn sub(&self, other: &Self) -> Self {
        let mut negated = other.clone();
        if !negated.is_zero() {
            negated.negative = !negated.negative;
        }
        self.add(&negated)
    }

    pub fn neg(&self) -> Self {
        let mut value = self.clone();
        if !value.is_zero() {
            value.negative = !value.negative;
        }
        value
    }

    pub fn mul(&self, other: &Self) -> Self {
        let mut limbs = vec![0u32; self.limbs.len() + other.limbs.len()];
        for (i, &left) in self.limbs.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &right) in other.limbs.iter().enumerate() {
                let index = i + j;
                let value = u64::from(limbs[index]) + u64::from(left) * u64::from(right) + carry;
                limbs[index] = value as u32;
                carry = value >> 32;
            }
            let mut index = i + other.limbs.len();
            while carry != 0 {
                let value = u64::from(limbs[index]) + carry;
                limbs[index] = value as u32;
                carry = value >> 32;
                index += 1;
            }
        }
        Self {
            negative: self.negative != other.negative,
            limbs,
        }
        .normalized()
    }

    /// Divide with truncation toward zero. The remainder has the dividend's
    /// sign, matching JavaScript BigInt semantics.
    pub fn div_rem(&self, other: &Self) -> Option<(Self, Self)> {
        if other.is_zero() {
            return None;
        }
        let mut dividend = self.clone();
        let mut divisor = other.clone();
        dividend.negative = false;
        divisor.negative = false;
        let mut quotient = Self::zero();
        let mut remainder = Self::zero();
        let bit_count = dividend.limbs.len() * 32;
        for bit in (0..bit_count).rev() {
            remainder.mul_small(2);
            if (dividend.limbs[bit / 32] >> (bit % 32)) & 1 != 0 {
                remainder.add_small(1);
            }
            if remainder.cmp(&divisor) != Ordering::Less {
                remainder = remainder.sub(&divisor);
                quotient.set_bit(bit);
            }
        }
        quotient.negative = self.negative != other.negative && !quotient.is_zero();
        remainder.negative = self.negative && !remainder.is_zero();
        Some((quotient.normalized(), remainder.normalized()))
    }

    pub fn cmp(&self, other: &Self) -> Ordering {
        match (self.negative, other.negative) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => cmp_abs(&self.limbs, &other.limbs),
            (true, true) => cmp_abs(&other.limbs, &self.limbs),
        }
    }

    /// Raise to a non-negative power. A negative exponent is a RangeError in
    /// JS BigInt, surfaced here as `Err`.
    pub fn pow(&self, exponent: &Self) -> Result<Self, &'static str> {
        if exponent.negative {
            return Err("BigInt exponent must be non-negative");
        }
        let mut result = Self::from_u64(1);
        let mut base = self.clone();
        // Only iterate up to the exponent's highest set bit: iterating the full
        // limb width would keep squaring `base` long past the top bit and blow
        // its size up exponentially.
        let bits = exponent.bit_length();
        for bit in 0..bits {
            if (exponent.limbs[bit / 32] >> (bit % 32)) & 1 == 1 {
                result = result.mul(&base);
            }
            if bit + 1 < bits {
                base = base.mul(&base);
            }
        }
        Ok(result)
    }

    /// Bitwise AND under infinite two's-complement semantics.
    pub fn bitand(&self, other: &Self) -> Self {
        self.bitwise(other, |a, b| a & b)
    }
    /// Bitwise OR under infinite two's-complement semantics.
    pub fn bitor(&self, other: &Self) -> Self {
        self.bitwise(other, |a, b| a | b)
    }
    /// Bitwise XOR under infinite two's-complement semantics.
    pub fn bitxor(&self, other: &Self) -> Self {
        self.bitwise(other, |a, b| a ^ b)
    }
    /// Bitwise NOT: `~x == -(x + 1)` (spec-exact for negatives too).
    pub fn bitnot(&self) -> Self {
        self.add(&Self::from_u64(1)).neg()
    }

    /// Left shift: `self << amount == self * 2^amount`.
    pub fn shl(&self, amount: u64) -> Self {
        self.mul(&Self::power_of_two(amount))
    }

    /// Arithmetic right shift: `self >> amount == floor(self / 2^amount)`.
    /// For negative values this rounds toward negative infinity, matching the
    /// JS `>>` operator on BigInt.
    pub fn shr(&self, amount: u64) -> Self {
        let divisor = Self::power_of_two(amount);
        let (quotient, remainder) = self
            .div_rem(&divisor)
            .expect("power of two divisor is never zero");
        if self.negative && !remainder.is_zero() {
            quotient.sub(&Self::from_u64(1))
        } else {
            quotient
        }
    }

    /// Interpret `self` as a two's-complement value `bits` wide and return the
    /// signed result (`BigInt.asIntN`).
    pub fn as_int_n(&self, bits: u64) -> Self {
        if bits == 0 {
            return Self::zero();
        }
        let modulus = Self::power_of_two(bits);
        let (_, mut rem) = self.div_rem(&modulus).expect("modulus is non-zero");
        // Map the remainder into `[0, 2^bits)`.
        if rem.negative {
            rem = rem.add(&modulus);
        }
        // Values at or above 2^(bits-1) wrap to the negative range.
        let half = Self::power_of_two(bits - 1);
        if rem.cmp(&half) != Ordering::Less {
            rem.sub(&modulus)
        } else {
            rem
        }
    }

    /// Interpret `self` as an unsigned two's-complement value `bits` wide
    /// (`BigInt.asUintN`): the non-negative remainder modulo `2^bits`.
    pub fn as_uint_n(&self, bits: u64) -> Self {
        if bits == 0 {
            return Self::zero();
        }
        let modulus = Self::power_of_two(bits);
        let (_, mut rem) = self.div_rem(&modulus).expect("modulus is non-zero");
        if rem.negative {
            rem = rem.add(&modulus);
        }
        rem
    }

    /// Return `self` as a `u64` if it fits in 64 bits (ignoring sign), else
    /// `None`. Used for shift/`asIntN` width arguments.
    pub fn to_u64(&self) -> Option<u64> {
        if self.limbs.len() > 2 {
            return None;
        }
        let low = u64::from(*self.limbs.first().unwrap_or(&0));
        let high = u64::from(*self.limbs.get(1).unwrap_or(&0));
        Some(low | (high << 32))
    }

    /// The position of the highest set bit plus one (0 for zero).
    fn bit_length(&self) -> usize {
        match self.limbs.last() {
            None => 0,
            Some(&top) => (self.limbs.len() - 1) * 32 + (32 - top.leading_zeros() as usize),
        }
    }

    /// `2^exponent` as a BigInt.
    fn power_of_two(exponent: u64) -> Self {
        let mut value = Self::zero();
        value.set_bit(exponent as usize);
        value
    }

    /// Apply a limb-wise bitwise operator under infinite two's-complement.
    fn bitwise(&self, other: &Self, op: impl Fn(u32, u32) -> u32) -> Self {
        let width = self.limbs.len().max(other.limbs.len()) + 1;
        let left = self.to_twos_complement(width);
        let right = other.to_twos_complement(width);
        let combined: Vec<u32> = left
            .iter()
            .zip(right.iter())
            .map(|(&a, &b)| op(a, b))
            .collect();
        Self::from_twos_complement(combined)
    }

    /// Sign-extend the magnitude into a `width`-limb two's-complement vector.
    fn to_twos_complement(&self, width: usize) -> Vec<u32> {
        let mut limbs = vec![0u32; width];
        for (slot, &limb) in limbs.iter_mut().zip(self.limbs.iter()) {
            *slot = limb;
        }
        if self.negative {
            // Negate across the full width: invert every limb, then add one.
            let mut carry = 1u64;
            for limb in limbs.iter_mut() {
                let value = u64::from(!*limb) + carry;
                *limb = value as u32;
                carry = value >> 32;
            }
        }
        limbs
    }

    /// Rebuild a sign-magnitude value from a two's-complement limb vector whose
    /// top limb carries the sign bit.
    fn from_twos_complement(mut limbs: Vec<u32>) -> Self {
        let negative = limbs.last().is_some_and(|&limb| limb >> 31 == 1);
        if negative {
            let mut carry = 1u64;
            for limb in limbs.iter_mut() {
                let value = u64::from(!*limb) + carry;
                *limb = value as u32;
                carry = value >> 32;
            }
        }
        Self { negative, limbs }.normalized()
    }

    fn from_u128(mut value: u128, negative: bool) -> Self {
        let mut limbs = Vec::new();
        while value != 0 {
            limbs.push(value as u32);
            value >>= 32;
        }
        Self { negative, limbs }.normalized()
    }
    fn normalized(mut self) -> Self {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
        if self.limbs.is_empty() {
            self.negative = false;
        }
        self
    }
    fn mul_small(&mut self, value: u32) {
        let mut carry = 0u64;
        for limb in &mut self.limbs {
            let product = u64::from(*limb) * u64::from(value) + carry;
            *limb = product as u32;
            carry = product >> 32;
        }
        if carry != 0 {
            self.limbs.push(carry as u32);
        }
    }
    fn add_small(&mut self, value: u32) {
        let mut carry = u64::from(value);
        for limb in &mut self.limbs {
            let sum = u64::from(*limb) + carry;
            *limb = sum as u32;
            carry = sum >> 32;
            if carry == 0 {
                return;
            }
        }
        if carry != 0 {
            self.limbs.push(carry as u32);
        }
    }
    fn set_bit(&mut self, bit: usize) {
        let limb = bit / 32;
        if self.limbs.len() <= limb {
            self.limbs.resize(limb + 1, 0);
        }
        self.limbs[limb] |= 1u32 << (bit % 32);
    }
}

fn cmp_abs(left: &[u32], right: &[u32]) -> Ordering {
    left.len()
        .cmp(&right.len())
        .then_with(|| left.iter().rev().cmp(right.iter().rev()))
}

fn add_abs(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut result = Vec::with_capacity(left.len().max(right.len()) + 1);
    let mut carry = 0u64;
    for i in 0..left.len().max(right.len()) {
        let sum =
            u64::from(*left.get(i).unwrap_or(&0)) + u64::from(*right.get(i).unwrap_or(&0)) + carry;
        result.push(sum as u32);
        carry = sum >> 32;
    }
    if carry != 0 {
        result.push(carry as u32);
    }
    result
}

fn sub_abs(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut result = Vec::with_capacity(left.len());
    let mut borrow = 0i64;
    for i in 0..left.len() {
        let difference = i64::from(left[i]) - i64::from(*right.get(i).unwrap_or(&0)) - borrow;
        result.push(difference as u32);
        borrow = i64::from(difference < 0);
    }
    result
}

impl core::fmt::Display for BigInt {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.is_zero() {
            return formatter.write_str("0");
        }
        let mut value = self.clone();
        value.negative = false;
        let mut digits = Vec::new();
        while !value.is_zero() {
            let mut remainder = 0u64;
            for limb in value.limbs.iter_mut().rev() {
                let current = (remainder << 32) | u64::from(*limb);
                *limb = (current / 10) as u32;
                remainder = current % 10;
            }
            value = value.normalized();
            digits.push((b'0' + remainder as u8) as char);
        }
        if self.negative {
            formatter.write_str("-")?;
        }
        for digit in digits.iter().rev() {
            formatter.write_str(&digit.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::BigInt;
    use core::cmp::Ordering;

    #[test]
    fn parses_and_formats_values_beyond_i64() {
        let value = BigInt::parse_decimal("18446744073709551616").unwrap();
        assert_eq!(value.to_string(), "18446744073709551616");
        assert_eq!(
            BigInt::parse_decimal("-18446744073709551616")
                .unwrap()
                .to_string(),
            "-18446744073709551616"
        );
    }

    #[test]
    fn signed_arithmetic_normalizes_zero() {
        let a = BigInt::parse_decimal("999999999999999999999").unwrap();
        let b = BigInt::parse_decimal("1").unwrap();
        assert_eq!(a.add(&b).to_string(), "1000000000000000000000");
        assert_eq!(a.sub(&a).to_string(), "0");
        assert_eq!(a.mul(&b).cmp(&a), Ordering::Equal);
    }

    #[test]
    fn division_truncates_and_keeps_remainder_sign() {
        let left = BigInt::parse_decimal("-17").unwrap();
        let right = BigInt::parse_decimal("5").unwrap();
        let (quotient, remainder) = left.div_rem(&right).unwrap();
        assert_eq!(quotient.to_string(), "-3");
        assert_eq!(remainder.to_string(), "-2");
    }

    fn big(text: &str) -> BigInt {
        BigInt::parse(text).unwrap()
    }

    #[test]
    fn parses_radix_prefixes_and_signs() {
        assert_eq!(big("0xff").to_string(), "255");
        assert_eq!(big("0o17").to_string(), "15");
        assert_eq!(big("0b1010").to_string(), "10");
        assert_eq!(big("-0x10").to_string(), "-16");
        assert_eq!(big("255").to_string(), "255");
        assert!(BigInt::parse("0x").is_err());
        assert!(BigInt::parse("12a").is_err());
    }

    #[test]
    fn powers_reject_negative_exponents() {
        assert_eq!(big("2").pow(&big("10")).unwrap().to_string(), "1024");
        assert_eq!(big("5").pow(&big("0")).unwrap().to_string(), "1");
        assert_eq!(big("-3").pow(&big("3")).unwrap().to_string(), "-27");
        assert!(big("2").pow(&big("-1")).is_err());
    }

    #[test]
    fn bitwise_follows_twos_complement_for_negatives() {
        assert_eq!(big("7").bitand(&big("3")).to_string(), "3");
        assert_eq!(big("12").bitor(&big("1")).to_string(), "13");
        assert_eq!(big("6").bitxor(&big("3")).to_string(), "5");
        assert_eq!(big("5").bitnot().to_string(), "-6");
        assert_eq!(big("-1").bitand(&big("5")).to_string(), "5");
        assert_eq!(big("-6").bitor(&big("3")).to_string(), "-5");
        assert_eq!(big("-8").bitxor(&big("-1")).to_string(), "7");
    }

    #[test]
    fn shifts_match_power_of_two_scaling() {
        assert_eq!(big("1").shl(4).to_string(), "16");
        assert_eq!(big("-8").shr(1).to_string(), "-4");
        assert_eq!(big("-7").shr(1).to_string(), "-4");
        assert_eq!(big("7").shr(1).to_string(), "3");
        assert_eq!(big("255").shl(0).to_string(), "255");
    }

    #[test]
    fn as_int_n_and_as_uint_n_wrap_like_the_spec() {
        assert_eq!(big("255").as_int_n(8).to_string(), "-1");
        assert_eq!(big("256").as_uint_n(8).to_string(), "0");
        assert_eq!(big("9").as_int_n(4).to_string(), "-7");
        assert_eq!(big("-1").as_uint_n(8).to_string(), "255");
    }

    #[test]
    fn from_f64_rejects_non_integers() {
        assert_eq!(BigInt::from_f64(42.0).unwrap().to_string(), "42");
        assert_eq!(BigInt::from_f64(-5.0).unwrap().to_string(), "-5");
        assert_eq!(
            BigInt::from_f64(9007199254740992.0).unwrap().to_string(),
            "9007199254740992"
        );
        assert!(BigInt::from_f64(1.5).is_err());
        assert!(BigInt::from_f64(f64::INFINITY).is_err());
    }
}
