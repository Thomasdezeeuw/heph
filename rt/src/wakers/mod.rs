//! Wakers implementation.
//!
//! # Implementation
//!
//! The implementation is fairly simple. All it does is set a bit in an
//! [`AtomicBitMap`] contained in an [`Arc`].

pub(crate) mod ring_waker;
pub(crate) mod shared;
#[cfg(test)]
mod tests;
