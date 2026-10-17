//! Protocol, curve math and config parsing for the NZXT Kraken 2023.
//!
//! Everything in this crate is `no_std` and allocation-free so the
//! freestanding `aioconf` binary can use it. Unit tests run on the host
//! with std.

#![cfg_attr(not(test), no_std)]

pub mod config;
pub mod curve;
pub mod proto;
