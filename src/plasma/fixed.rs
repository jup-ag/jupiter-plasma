use std::{
    fmt::{Debug, Display, Formatter},
    ops::{Add, AddAssign, Mul, Sub},
};

use borsh::{BorshDeserialize, BorshSerialize};
use bytemuck::{Pod, Zeroable};

type FixedI80F48 = fixed::types::I80F48;

#[derive(Clone, Copy, Zeroable, Pod, BorshDeserialize, BorshSerialize)]
#[repr(C)]
#[borsh(crate = "borsh")]
pub struct I80F48 {
    inner: i128,
}

impl I80F48 {
    pub const ZERO: Self = Self { inner: 0 };

    pub fn from_num(value: u64) -> Self {
        let value = FixedI80F48::from_num(value);
        Self {
            inner: value.to_bits(),
        }
    }

    pub fn from_fraction(numerator: u64, denominator: u64) -> Self {
        let value = FixedI80F48::from_num(numerator) / FixedI80F48::from_num(denominator);
        Self {
            inner: value.to_bits(),
        }
    }

    pub fn floor(&self) -> u64 {
        let value = FixedI80F48::from_bits(self.inner);
        value.floor().to_num()
    }

    pub fn to_bits(&self) -> i128 {
        self.inner
    }

    pub fn from_bits(bits: i128) -> Self {
        Self { inner: bits }
    }
}

impl PartialEq for I80F48 {
    fn eq(&self, rhs: &Self) -> bool {
        let lhs = FixedI80F48::from_bits(self.inner);
        let rhs = FixedI80F48::from_bits(rhs.inner);
        lhs == rhs
    }
}

impl PartialOrd for I80F48 {
    fn partial_cmp(&self, rhs: &Self) -> Option<std::cmp::Ordering> {
        let lhs = FixedI80F48::from_bits(self.inner);
        let rhs = FixedI80F48::from_bits(rhs.inner);
        lhs.partial_cmp(&rhs)
    }
}

impl AddAssign for I80F48 {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Add for I80F48 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        let lhs = FixedI80F48::from_bits(self.inner);
        let rhs = FixedI80F48::from_bits(rhs.inner);
        let sum = lhs + rhs;
        Self {
            inner: sum.to_bits(),
        }
    }
}

impl Sub for I80F48 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        let lhs = FixedI80F48::from_bits(self.inner);
        let rhs = FixedI80F48::from_bits(rhs.inner);
        let diff = lhs - rhs;
        Self {
            inner: diff.to_bits(),
        }
    }
}

impl Mul for I80F48 {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        let lhs = FixedI80F48::from_bits(self.inner);
        let rhs = FixedI80F48::from_bits(rhs.inner);
        let product = lhs * rhs;
        Self {
            inner: product.to_bits(),
        }
    }
}

impl Display for I80F48 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = FixedI80F48::from_bits(self.inner);
        write!(f, "{}", value)
    }
}

impl Debug for I80F48 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = FixedI80F48::from_bits(self.inner);
        write!(f, "{:?}", value)
    }
}
