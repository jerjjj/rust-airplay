// SPDX-License-Identifier: GPL-3.0-or-later
//! The media streams a client sends after `SETUP`.
//!
//! Upstream splits this work in two: the protocol side receives datagrams,
//! decrypts them and puts them back in order (`lib/raop_rtp.c`,
//! `lib/raop_rtp_mirror.c`, `lib/raop_buffer.c`), and the renderer side decodes
//! and plays them (`renderers/`, GStreamer, see
//! `docs/ADR-0003-media-backend.md`). This module is the first one: it belongs to
//! the protocol core, depends on no media library, and hands frames to whoever
//! renders them.
//!
//! It grows one slice at a time, each with its own tests
//! (`docs/S4_WORK_PLAN.md`):
//!
//! - [`audio`]: reading the audio datagrams a client sends and delivering their
//!   payloads in sequence order, with a bounded window and a record of what went
//!   missing.

pub mod audio;
pub mod control;
pub mod timing;
