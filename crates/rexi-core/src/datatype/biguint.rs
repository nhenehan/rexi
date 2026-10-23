//! A small unsigned integer type with no size limit ("big integer"), with just enough
//! operations for EXI Unsigned Integer values larger than `u64`.
//! [EXI 7.1.6](https://www.w3.org/TR/exi/#encodingUnsignedInteger) puts no upper limit
//! on them.
//!
//! The number is stored as 32-bit pieces called limbs (each limb is one "digit" in
//! base 2^32). Every operation here works one limb at a time and carries the overflow
//! into the next limb, the same way long multiplication and long division work on
//! paper.

use crate::prelude::*;
use core::cmp::Ordering;
use core::fmt;

/// Base-2^32 limbs in little-endian order (lowest limb first). Rule that always
/// holds: there are no zero limbs at the end, so zero is an empty vector. This keeps
/// one form per value, which makes `==`, `Hash` and the length check in `cmp` correct.
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct BigUint {
    limbs: Vec<u32>,
}

impl BigUint {
    /// The number zero (no limbs).
    pub const fn zero() -> Self {
        Self { limbs: Vec::new() }
    }

    /// The value of a `u64`.
    pub fn from_u64(v: u64) -> Self {
        let mut n = Self {
            limbs: vec![v as u32, (v >> 32) as u32],
        };
        n.normalize();
        n
    }

    /// The value of a `u128`.
    pub fn from_u128(v: u128) -> Self {
        let mut n = Self {
            limbs: vec![
                v as u32,
                (v >> 32) as u32,
                (v >> 64) as u32,
                (v >> 96) as u32,
            ],
        };
        n.normalize();
        n
    }

    /// Whether the number is zero.
    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    /// The value as a `u64`, or `None` when it does not fit.
    pub fn to_u64(&self) -> Option<u64> {
        match self.limbs.len() {
            0 => Some(0),
            1 => Some(u64::from(self.limbs[0])),
            2 => Some(u64::from(self.limbs[0]) | (u64::from(self.limbs[1]) << 32)),
            _ => None,
        }
    }

    /// The limbs, lowest first, with no zero limbs at the end (zero is empty).
    pub fn limbs(&self) -> &[u32] {
        &self.limbs
    }

    fn normalize(&mut self) {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
    }

    /// Parses a string of ASCII decimal digits (no sign, no spaces).
    pub fn from_decimal_str(s: &str) -> Option<Self> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut n = Self::zero();
        // Read up to 9 digits at a time, since 10^9 < 2^32 fits in one limb. For each
        // chunk: value = value * 10^(digits in chunk) + chunk.
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let take = (bytes.len() - i).min(9);
            let mut chunk = 0u32;
            let mut mul = 1u32;
            for &b in &bytes[i..i + take] {
                chunk = chunk * 10 + u32::from(b - b'0');
                mul *= 10;
            }
            n.mul_small_add(mul, chunk);
            i += take;
        }
        Some(n)
    }

    /// `self = self * m + a`.
    pub fn mul_small_add(&mut self, m: u32, a: u32) {
        let mut carry = u64::from(a);
        for limb in &mut self.limbs {
            let v = u64::from(*limb) * u64::from(m) + carry;
            *limb = v as u32;
            carry = v >> 32;
        }
        if carry != 0 {
            self.limbs.push(carry as u32);
        }
        self.normalize();
    }

    /// Divides in place by a small divisor and returns the remainder. Works from the
    /// top limb down, like long division.
    pub fn div_small_rem(&mut self, d: u32) -> u32 {
        let mut rem = 0u64;
        for limb in self.limbs.iter_mut().rev() {
            let cur = (rem << 32) | u64::from(*limb);
            *limb = (cur / u64::from(d)) as u32;
            rem = cur % u64::from(d);
        }
        self.normalize();
        rem as u32
    }

    /// The number in base 10, with no leading zeros (`"0"` for zero).
    pub fn to_decimal_string(&self) -> String {
        if self.is_zero() {
            return "0".to_string();
        }
        let mut n = self.clone();
        let mut chunks = Vec::new();
        while !n.is_zero() {
            chunks.push(n.div_small_rem(1_000_000_000));
        }
        let mut s = String::new();
        if let Some((last, rest)) = chunks.split_last() {
            s.push_str(&last.to_string());
            for c in rest.iter().rev() {
                s.push_str(&format!("{c:09}"));
            }
        }
        s
    }

    /// Low 7 bits, for writing the next varint group.
    pub fn low7(&self) -> u8 {
        (self.limbs.first().copied().unwrap_or(0) & 0x7F) as u8
    }

    /// Shifts right by 7 bits in place. The 7 bits pushed out of each limb move into
    /// the top of the limb below it (`carry << 25`, since 32 - 7 = 25).
    pub fn shr7(&mut self) {
        let mut carry = 0u32;
        for limb in self.limbs.iter_mut().rev() {
            let v = *limb;
            *limb = (v >> 7) | (carry << 25);
            carry = v & 0x7F;
        }
        self.normalize();
    }

    /// Builds a value from 7-bit groups, least significant group first.
    pub fn from_7bit_groups(groups: &[u8]) -> Self {
        let mut n = Self::zero();
        for &g in groups.iter().rev() {
            n.shl7_or(u32::from(g & 0x7F));
        }
        n
    }

    /// Shifts left by 7 bits and puts `low` in the 7 bits that open up at the bottom.
    fn shl7_or(&mut self, low: u32) {
        let mut carry = low;
        for limb in &mut self.limbs {
            let v = *limb;
            *limb = (v << 7) | carry;
            carry = v >> 25;
        }
        if carry != 0 {
            self.limbs.push(carry);
        }
        self.normalize();
    }

    /// Reverses the decimal digits (used for the fractional part of decimals,
    /// [EXI 7.1.3](https://www.w3.org/TR/exi/#encodingDecimal)). Zeros that end up in
    /// front after reversing are dropped, so 100 becomes 1.
    #[must_use]
    pub fn reverse_decimal_digits(&self) -> Self {
        let s = self.to_decimal_string();
        let rev: String = s.chars().rev().collect();
        let trimmed = rev.trim_start_matches('0');
        if trimmed.is_empty() {
            Self::zero()
        } else {
            Self::from_decimal_str(trimmed).unwrap_or_default()
        }
    }

    /// `self + 1`
    #[must_use]
    pub fn add_one(&self) -> Self {
        let mut n = self.clone();
        n.mul_small_add(1, 1);
        n
    }

    /// `self - 1`, saturating at zero.
    #[must_use]
    pub fn sub_one(&self) -> Self {
        if self.is_zero() {
            return Self::zero();
        }
        let mut n = self.clone();
        let mut i = 0;
        loop {
            if n.limbs[i] == 0 {
                n.limbs[i] = u32::MAX;
                i += 1;
            } else {
                n.limbs[i] -= 1;
                break;
            }
        }
        n.normalize();
        n
    }
}

impl PartialOrd for BigUint {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BigUint {
    fn cmp(&self, other: &Self) -> Ordering {
        self.limbs
            .len()
            .cmp(&other.limbs.len())
            .then_with(|| self.limbs.iter().rev().cmp(other.limbs.iter().rev()))
    }
}

impl From<u64> for BigUint {
    fn from(v: u64) -> Self {
        Self::from_u64(v)
    }
}

impl fmt::Debug for BigUint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BigUint({})", self.to_decimal_string())
    }
}

impl fmt::Display for BigUint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_decimal_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_round_trip() {
        for s in [
            "0",
            "1",
            "4294967295",
            "4294967296",
            "18446744073709551615",
            "18446744073709551616",
            "123456789012345678901234567890",
        ] {
            let n = BigUint::from_decimal_str(s);
            assert!(n.is_some(), "{s}");
            assert_eq!(n.unwrap_or_default().to_decimal_string(), s);
        }
        assert!(BigUint::from_decimal_str("").is_none());
        assert!(BigUint::from_decimal_str("12a").is_none());
    }

    #[test]
    fn u64_conversions() {
        assert_eq!(BigUint::from_u64(u64::MAX).to_u64(), Some(u64::MAX));
        assert_eq!(BigUint::from_u128(1u128 << 64).to_u64(), None);
        assert_eq!(BigUint::from_u64(0), BigUint::zero());
    }

    #[test]
    fn seven_bit_groups_round_trip() {
        let n = BigUint::from_decimal_str("123456789012345678901234567890").unwrap_or_default();
        let mut groups = Vec::new();
        let mut m = n.clone();
        while !m.is_zero() {
            groups.push(m.low7());
            m.shr7();
        }
        assert_eq!(BigUint::from_7bit_groups(&groups), n);
    }

    #[test]
    fn reverse_digits() {
        assert_eq!(
            BigUint::from_u64(100).reverse_decimal_digits(),
            BigUint::from_u64(1)
        );
        assert_eq!(
            BigUint::from_u64(5).reverse_decimal_digits(),
            BigUint::from_u64(5)
        );
        assert_eq!(
            BigUint::from_u64(59).reverse_decimal_digits(),
            BigUint::from_u64(95)
        );
    }

    #[test]
    fn ordering_and_inc_dec() {
        let a = BigUint::from_u64(u64::MAX);
        let b = a.add_one();
        assert!(b > a);
        assert_eq!(b.sub_one(), a);
        assert_eq!(BigUint::zero().sub_one(), BigUint::zero());
    }
}
