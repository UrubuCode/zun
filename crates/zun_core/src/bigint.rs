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
    pub fn zero() -> Self { Self { negative: false, limbs: Vec::new() } }

    pub fn from_u64(value: u64) -> Self {
        let mut limbs = vec![value as u32];
        if value >> 32 != 0 { limbs.push((value >> 32) as u32); }
        Self { negative: false, limbs }.normalized()
    }

    pub fn from_i64(value: i64) -> Self {
        if value < 0 {
            let magnitude = (-(value as i128)) as u128;
            Self::from_u128(magnitude, true)
        } else {
            Self::from_u128(value as u128, false)
        }
    }

    pub fn parse_decimal(text: &str) -> Result<Self, &'static str> {
        let bytes = text.as_bytes();
        if bytes.is_empty() { return Err("empty bigint literal"); }
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

    pub fn is_zero(&self) -> bool { self.limbs.is_empty() }
    pub fn is_negative(&self) -> bool { self.negative }
    pub fn add(&self, other: &Self) -> Self {
        match (self.negative, other.negative) {
            (a, b) if a == b => Self { negative: a, limbs: add_abs(&self.limbs, &other.limbs) }.normalized(),
            _ => match cmp_abs(&self.limbs, &other.limbs) {
                Ordering::Equal => Self::zero(),
                Ordering::Greater => Self { negative: self.negative, limbs: sub_abs(&self.limbs, &other.limbs) }.normalized(),
                Ordering::Less => Self { negative: other.negative, limbs: sub_abs(&other.limbs, &self.limbs) }.normalized(),
            },
        }
    }

    pub fn sub(&self, other: &Self) -> Self {
        let mut negated = other.clone();
        if !negated.is_zero() { negated.negative = !negated.negative; }
        self.add(&negated)
    }

    pub fn neg(&self) -> Self {
        let mut value = self.clone();
        if !value.is_zero() { value.negative = !value.negative; }
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
        Self { negative: self.negative != other.negative, limbs }.normalized()
    }

    /// Divide with truncation toward zero. The remainder has the dividend's
    /// sign, matching JavaScript BigInt semantics.
    pub fn div_rem(&self, other: &Self) -> Option<(Self, Self)> {
        if other.is_zero() { return None; }
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

    fn from_u128(mut value: u128, negative: bool) -> Self {
        let mut limbs = Vec::new();
        while value != 0 { limbs.push(value as u32); value >>= 32; }
        Self { negative, limbs }.normalized()
    }
    fn normalized(mut self) -> Self {
        while self.limbs.last() == Some(&0) { self.limbs.pop(); }
        if self.limbs.is_empty() { self.negative = false; }
        self
    }
    fn mul_small(&mut self, value: u32) {
        let mut carry = 0u64;
        for limb in &mut self.limbs {
            let product = u64::from(*limb) * u64::from(value) + carry;
            *limb = product as u32;
            carry = product >> 32;
        }
        if carry != 0 { self.limbs.push(carry as u32); }
    }
    fn add_small(&mut self, value: u32) {
        let mut carry = u64::from(value);
        for limb in &mut self.limbs {
            let sum = u64::from(*limb) + carry;
            *limb = sum as u32;
            carry = sum >> 32;
            if carry == 0 { return; }
        }
        if carry != 0 { self.limbs.push(carry as u32); }
    }
    fn set_bit(&mut self, bit: usize) {
        let limb = bit / 32;
        if self.limbs.len() <= limb { self.limbs.resize(limb + 1, 0); }
        self.limbs[limb] |= 1u32 << (bit % 32);
    }
}

fn cmp_abs(left: &[u32], right: &[u32]) -> Ordering {
    left.len().cmp(&right.len()).then_with(|| left.iter().rev().cmp(right.iter().rev()))
}

fn add_abs(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut result = Vec::with_capacity(left.len().max(right.len()) + 1);
    let mut carry = 0u64;
    for i in 0..left.len().max(right.len()) {
        let sum = u64::from(*left.get(i).unwrap_or(&0)) + u64::from(*right.get(i).unwrap_or(&0)) + carry;
        result.push(sum as u32);
        carry = sum >> 32;
    }
    if carry != 0 { result.push(carry as u32); }
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
        if self.is_zero() { return formatter.write_str("0"); }
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
        if self.negative { formatter.write_str("-")?; }
        for digit in digits.iter().rev() { formatter.write_str(&digit.to_string())?; }
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
        assert_eq!(BigInt::parse_decimal("-18446744073709551616").unwrap().to_string(), "-18446744073709551616");
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
}
