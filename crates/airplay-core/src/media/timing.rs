// SPDX-License-Identifier: GPL-3.0-or-later
//! The timing port of a session.
//!
//! A client and a receiver agree on how their clocks relate by exchanging timing
//! packets on their own port, which `SETUP` negotiates. Upstream sends a
//! thirty-two byte request and reads three values out of the answer
//! (`upstream/UxPlay@3dbf7cee:lib/raop_ntp.c:578-700`):
//!
//! ```text
//! request[0]      0x80
//! request[1]      0xd2, whose type is 0x52
//! request[2..3]   0x00 0x07
//! request[24..31] the local time when the request went out
//! ```
//!
//! and out of the response it reads
//!
//! ```text
//! response[8..11]   t1, 32 bits, big endian
//! response[16..19]  t2, 32 bits, the client's receive time
//! response[24..27]  t3, 32 bits, the client's transmit time
//! ```
//!
//! This module is the packet layer of that exchange: it builds the request and
//! reads what the answer carries. The clock arithmetic on top of it is *not*
//! here yet, and deliberately so: upstream adjusts the remote values with
//! `raop_ntp_adjust_remote_timestamp_offset`, rejects duplicates and bogus
//! packets, and folds in a `sync_delay`, all of which has to be read out of the
//! source rather than guessed at. `docs/S4_WORK_PLAN.md` records that as the next
//! slice.

use crate::ntp;

/// Length of a timing request, which upstream sends in full
/// (`raop_ntp.c:580-581`).
pub const TIMING_REQUEST_LEN: usize = 32;

/// The first two bytes of a request (`raop_ntp.c:580`).
const REQUEST_HEADER: [u8; 4] = [0x80, 0xd2, 0x00, 0x07];

/// Where a request carries the time it was sent (`raop_ntp.c:616`).
const REQUEST_TRANSMIT_OFFSET: usize = 24;

/// Type of a timing response, which is a request whose type bit changed
/// (`raop_ntp.c:580` and the branch that reads the answer).
pub const RESPONSE_TYPE: u8 = 0x53;

/// Where a response carries the three values upstream reads
/// (`raop_ntp.c:656-665`).
const RESPONSE_T1_OFFSET: usize = 8;
const RESPONSE_T2_OFFSET: usize = 16;
const RESPONSE_T3_OFFSET: usize = 24;

/// Shortest response this module can read.
pub const RESPONSE_MIN_LEN: usize = RESPONSE_T3_OFFSET + 4;

/// Builds the timing request a receiver sends to a client's timing port.
///
/// `send_time` is the local time in the nanosecond form [`crate::ntp`] works
/// with; it is written where upstream writes it, so a response can be matched to
/// the request that caused it.
///
/// # Errors
///
/// Returns the error of [`ntp::unix_nanos_to_timestamp`] when the time cannot be
/// represented in the NTP era this port uses.
pub fn build_request(send_time: u64) -> Result<[u8; TIMING_REQUEST_LEN], crate::wire::ParseError> {
    let mut request = [0u8; TIMING_REQUEST_LEN];
    request[..REQUEST_HEADER.len()].copy_from_slice(&REQUEST_HEADER);
    let timestamp = ntp::unix_nanos_to_timestamp(send_time)?;
    request[REQUEST_TRANSMIT_OFFSET..REQUEST_TRANSMIT_OFFSET + 8].copy_from_slice(&timestamp);
    Ok(request)
}

/// The values a timing response carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimingResponse {
    /// 32 bits at offset eight, the value upstream keeps as `t1`.
    pub t1: u32,
    /// 32 bits at offset sixteen, the client's receive time.
    pub t2: u32,
    /// 32 bits at offset twenty-four, the client's transmit time.
    pub t3: u32,
}

/// Why a datagram could not be read as a timing response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingError {
    /// Shorter than the values it has to carry.
    TooShort,
    /// Not an answer to a timing request.
    NotAResponse {
        /// Type the datagram carried, from `packet[1] & !0x80`.
        kind: u8,
    },
}

impl std::fmt::Display for TimingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(formatter, "the timing response is too short"),
            Self::NotAResponse { kind } => {
                write!(formatter, "type {kind:#04x} is not a timing response")
            }
        }
    }
}

impl std::error::Error for TimingError {}

/// Reads a timing response.
///
/// # Errors
///
/// Returns [`TimingError`] for a datagram too short to carry the values, or one
/// whose type is not [`RESPONSE_TYPE`].
pub fn parse_response(datagram: &[u8]) -> Result<TimingResponse, TimingError> {
    let Some(&second) = datagram.get(1) else {
        return Err(TimingError::TooShort);
    };
    if second & !0x80 != RESPONSE_TYPE {
        return Err(TimingError::NotAResponse {
            kind: second & !0x80,
        });
    }
    if datagram.len() < RESPONSE_MIN_LEN {
        return Err(TimingError::TooShort);
    }
    Ok(TimingResponse {
        t1: read_u32(&datagram[RESPONSE_T1_OFFSET..]),
        t2: read_u32(&datagram[RESPONSE_T2_OFFSET..]),
        t3: read_u32(&datagram[RESPONSE_T3_OFFSET..]),
    })
}

fn read_u32(bytes: &[u8]) -> u32 {
    let mut value = [0u8; 4];
    value.copy_from_slice(&bytes[..4]);
    u32::from_be_bytes(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_looks_like_the_one_upstream_sends() {
        // A Unix time in this era, with a fraction so the encoding is exercised.
        let send_time = 1_700_000_000 * ntp::NANOS_PER_SECOND + 123_456_789;
        let request = build_request(send_time).expect("request");
        assert_eq!(&request[..4], &[0x80, 0xd2, 0x00, 0x07], "header");
        assert_eq!(request.len(), 32);
        assert!(
            request[4..24].iter().all(|byte| *byte == 0),
            "everything before the transmit time stays clear"
        );
        // The transmit time comes back within the precision the format has: the
        // NTP fraction is 32 bits, which cannot hold every nanosecond.
        let mut timestamp = [0u8; 8];
        timestamp.copy_from_slice(&request[24..32]);
        let decoded = ntp::timestamp_to_unix_nanos(&timestamp).expect("timestamp");
        let difference = decoded.abs_diff(send_time);
        assert!(
            difference <= 2,
            "the round trip is off by {difference} ns, more than the format allows"
        );
    }

    #[test]
    fn reads_the_response_values_upstream_reads() {
        let mut response = vec![0u8; 32];
        response[1] = RESPONSE_TYPE | 0x80;
        response[8..12].copy_from_slice(&0x1111_1111u32.to_be_bytes());
        response[16..20].copy_from_slice(&0x2222_2222u32.to_be_bytes());
        response[24..28].copy_from_slice(&0x3333_3333u32.to_be_bytes());
        assert_eq!(
            parse_response(&response).expect("response"),
            TimingResponse {
                t1: 0x1111_1111,
                t2: 0x2222_2222,
                t3: 0x3333_3333,
            }
        );
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        assert_eq!(parse_response(&[]), Err(TimingError::TooShort));
        assert_eq!(parse_response(&[0x80]), Err(TimingError::TooShort));
        // A request is not a response.
        let request = build_request(1_700_000_000 * ntp::NANOS_PER_SECOND).expect("request");
        assert_eq!(
            parse_response(&request),
            Err(TimingError::NotAResponse { kind: 0x52 })
        );
        // The right type, but nothing to read in it.
        assert_eq!(
            parse_response(&[0x80, RESPONSE_TYPE | 0x80]),
            Err(TimingError::TooShort)
        );
    }
}
