// SPDX-License-Identifier: GPL-3.0-or-later
//! Protocol primitives for an UxPlay-compatible AirPlay receiver.
//!
//! Network input is untrusted. Parsers must validate lengths before allocation,
//! preserve wire byte order explicitly, and return errors instead of panicking.
//! Media playback and platform APIs belong in separate backends.
