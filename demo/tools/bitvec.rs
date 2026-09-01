//! A fixed-width bit vector with the semantics RTL people expect.
//!
//! Written the way a hardware modelling helper actually gets written:
//! wrapping arithmetic at the declared width, explicit sign handling,
//! and no silent promotion to a wider type. Rust's integer types stop at
//! 128 bits and panic on overflow in debug builds, neither of which
//! matches how a `logic [W-1:0]` behaves.
//!
//! Standalone by design -- it is not part of the reticle build, it
//! is here to be read and edited. To run its tests:
//!
//! ```text
//! rustc --test bitvec.rs -o /tmp/bitvec && /tmp/bitvec
//! ```

use std::fmt;

/// A `width`-bit unsigned value stored little-endian in 64-bit limbs.
#[derive(Clone, PartialEq, Eq)]
pub struct BitVec {
    width: usize,
    limbs: Vec<u64>,
}

impl BitVec {
    /// All-zero vector of the given width. Width 0 is rejected: a
    /// zero-width signal is a modelling mistake, not a useful value.
    pub fn zero(width: usize) -> Self {
        assert!(width > 0, "bit vector width must be positive");
        BitVec {
            width,
            limbs: vec![0; width.div_ceil(64)],
        }
    }

    pub fn from_u64(width: usize, value: u64) -> Self {
        let mut bv = BitVec::zero(width);
        bv.limbs[0] = value;
        bv.mask_off();
        bv
    }

    pub fn width(&self) -> usize {
        self.width
    }

    /// Clear the bits above `width` in the top limb, so every operation
    /// can leave the vector normalised without each caller remembering.
    fn mask_off(&mut self) {
        let spare = self.limbs.len() * 64 - self.width;
        if spare > 0 {
            let last = self.limbs.len() - 1;
            self.limbs[last] &= u64::MAX >> spare;
        }
    }

    pub fn get(&self, index: usize) -> bool {
        assert!(index < self.width, "bit {} out of range", index);
        (self.limbs[index / 64] >> (index % 64)) & 1 == 1
    }

    pub fn set(&mut self, index: usize, value: bool) {
        assert!(index < self.width, "bit {} out of range", index);
        let bit = 1u64 << (index % 64);
        if value {
            self.limbs[index / 64] |= bit;
        } else {
            self.limbs[index / 64] &= !bit;
        }
    }

    /// Addition that wraps at `width`, like the hardware would.
    pub fn wrapping_add(&self, other: &BitVec) -> BitVec {
        assert_eq!(self.width, other.width, "width mismatch");
        let mut out = BitVec::zero(self.width);
        let mut carry = 0u64;
        for i in 0..self.limbs.len() {
            let (sum, c1) = self.limbs[i].overflowing_add(other.limbs[i]);
            let (sum, c2) = sum.overflowing_add(carry);
            out.limbs[i] = sum;
            carry = u64::from(c1 || c2);
        }
        out.mask_off();
        out
    }

    /// Two's-complement negation, also wrapping.
    pub fn negate(&self) -> BitVec {
        let mut inverted = BitVec::zero(self.width);
        for (i, limb) in self.limbs.iter().enumerate() {
            inverted.limbs[i] = !limb;
        }
        inverted.mask_off();
        inverted.wrapping_add(&BitVec::from_u64(self.width, 1))
    }

    /// Interpret as signed and widen to `i128`, for comparisons against
    /// an expected value in a test.
    pub fn as_i128(&self) -> i128 {
        assert!(self.width <= 127, "width {} does not fit i128", self.width);
        let magnitude = self.as_u128();
        if self.get(self.width - 1) {
            magnitude as i128 - (1i128 << self.width)
        } else {
            magnitude as i128
        }
    }

    pub fn as_u128(&self) -> u128 {
        assert!(self.width <= 128, "width {} does not fit u128", self.width);
        let mut out = 0u128;
        for (i, limb) in self.limbs.iter().enumerate().take(2) {
            out |= u128::from(*limb) << (64 * i);
        }
        out
    }

    pub fn count_ones(&self) -> u32 {
        self.limbs.iter().map(|l| l.count_ones()).sum()
    }
}

impl fmt::Display for BitVec {
    /// Verilog-style sized literal, e.g. `8'b0000_1111`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}'b", self.width)?;
        for i in (0..self.width).rev() {
            write!(f, "{}", u8::from(self.get(i)))?;
            if i % 4 == 0 && i != 0 {
                write!(f, "_")?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for BitVec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addition_wraps_at_the_declared_width() {
        let a = BitVec::from_u64(8, 0xFF);
        let b = BitVec::from_u64(8, 1);
        assert_eq!(a.wrapping_add(&b), BitVec::zero(8));
    }

    #[test]
    fn construction_truncates_to_width() {
        // 0x1FF does not fit in 8 bits; the high bit must be dropped
        // rather than silently widening the value.
        assert_eq!(BitVec::from_u64(8, 0x1FF).as_u128(), 0xFF);
    }

    #[test]
    fn negation_is_twos_complement() {
        assert_eq!(BitVec::from_u64(8, 1).negate().as_i128(), -1);
        assert_eq!(BitVec::from_u64(8, 0).negate().as_i128(), 0);
        // The most negative value negates to itself, as in hardware.
        assert_eq!(BitVec::from_u64(8, 0x80).negate().as_i128(), -128);
    }

    #[test]
    fn wide_vectors_carry_between_limbs() {
        let a = BitVec::from_u64(96, u64::MAX);
        let one = BitVec::from_u64(96, 1);
        let sum = a.wrapping_add(&one);
        assert_eq!(sum.as_u128(), 1u128 << 64);
        assert_eq!(sum.count_ones(), 1);
    }

    #[test]
    fn display_uses_verilog_literal_syntax() {
        assert_eq!(BitVec::from_u64(8, 0x0F).to_string(), "8'b0000_1111");
    }
}
