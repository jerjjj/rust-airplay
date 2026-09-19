#![allow(missing_docs)]
// SPDX-License-Identifier: GPL-3.0-or-later
//! NTP wire timestamp conversion used by AirPlay timing and media clocks.

use crate::wire::ParseError;

pub const NTP_UNIX_EPOCH_SECONDS: u64 = 2_208_988_800;
pub const NANOS_PER_SECOND: u64 = 1_000_000_000;

pub fn timestamp_to_unix_nanos(bytes: &[u8]) -> Result<u64, ParseError> {
    if bytes.len() < 8 {
        return Err(ParseError::Incomplete);
    }
    let seconds = u32::from_be_bytes(
        bytes[..4]
            .try_into()
            .map_err(|_| ParseError::Invalid("NTP seconds"))?,
    ) as u64;
    let fraction = u32::from_be_bytes(
        bytes[4..8]
            .try_into()
            .map_err(|_| ParseError::Invalid("NTP fraction"))?,
    ) as u64;
    if seconds < NTP_UNIX_EPOCH_SECONDS {
        return Err(ParseError::Invalid("NTP timestamp before Unix epoch"));
    }
    let unix_seconds = seconds - NTP_UNIX_EPOCH_SECONDS;
    Ok(unix_seconds
        .saturating_mul(NANOS_PER_SECOND)
        .saturating_add((fraction.saturating_mul(NANOS_PER_SECOND)) >> 32))
}

pub fn unix_nanos_to_timestamp(nanos: u64) -> [u8; 8] {
    let seconds = nanos / NANOS_PER_SECOND + NTP_UNIX_EPOCH_SECONDS;
    let fraction = ((nanos % NANOS_PER_SECOND) << 32) / NANOS_PER_SECOND;
    let mut output = [0; 8];
    output[..4].copy_from_slice(&(seconds as u32).to_be_bytes());
    output[4..].copy_from_slice(&(fraction as u32).to_be_bytes());
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trips_ntp_epoch_conversion() {
        let timestamp = unix_nanos_to_timestamp(1_700_000_000_123_456_789);
        let decoded = timestamp_to_unix_nanos(&timestamp).unwrap();
        assert!((decoded as i128 - 1_700_000_000_123_456_789i128).abs() < 2);
    }
    #[test]
    fn rejects_short_and_pre_epoch_values() {
        assert_eq!(
            timestamp_to_unix_nanos(&[0; 7]),
            Err(ParseError::Incomplete)
        );
        assert_eq!(
            timestamp_to_unix_nanos(&[0; 8]),
            Err(ParseError::Invalid("NTP timestamp before Unix epoch"))
        );
    }
}
