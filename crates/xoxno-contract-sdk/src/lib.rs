#![no_std]
#![doc = include_str!("../README.md")]

#[cfg(any(test, feature = "testutils"))]
extern crate std;

pub mod lending;
pub mod networks;

pub use lending::{LendingAddresses, Position, XoxnoLending};

#[cfg(any(test, feature = "testutils"))]
pub mod testutils;
