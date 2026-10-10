//! Fast, deterministic hashing for vertex welding.
//!
//! The standard SipHash is several times slower on the millions of fixed-size keys a high
//! poly mesh produces, and its random seed is not needed: keys are not attacker-chosen.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Multiplicative hasher in the style of rustc's FxHash.
#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher {
    hash: u64,
}

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for chunk in chunks {
            self.add(u64::from_le_bytes(*chunk));
        }
        for &byte in rest {
            self.add(u64::from(byte));
        }
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// `HashMap` using [`FxHasher`].
pub(crate) type FxHashMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

/// Bit pattern of a float, with `-0.0` mapped to `0.0` so equal values weld.
#[inline]
pub(crate) fn float_key(value: f32) -> u32 {
    if value == 0.0 { 0 } else { value.to_bits() }
}

/// Bit patterns of a position.
#[inline]
pub(crate) fn position_key(position: [f32; 3]) -> [u32; 3] {
    position.map(float_key)
}
