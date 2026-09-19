// SPDX-License-Identifier: GPL-3.0-or-later
//! Protocol primitives for an UxPlay-compatible AirPlay receiver.
//!
//! Network input is untrusted. Parsers validate lengths before allocation,
//! preserve wire byte order explicitly, and return errors instead of panicking.

pub mod mirror;
pub mod ntp;
pub mod rtp;
pub mod wire;
