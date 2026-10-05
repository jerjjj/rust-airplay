// SPDX-License-Identifier: GPL-3.0-or-later
//! Protocol primitives for an UxPlay-compatible AirPlay receiver.

pub mod control;
pub mod crypto;
pub mod discovery;
pub mod fairplay;
mod fairplay_tables;
pub mod mirror;
pub mod ntp;
pub mod pairing;
mod playfair;
mod playfair_tables;
pub mod plist;
pub mod rtp;
pub mod server;
pub mod srp;
pub mod wire;
