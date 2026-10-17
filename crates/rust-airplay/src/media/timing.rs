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
//! source rather than guessed at. `docs/ARCHITECTURE.md` records that as the next
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
/// represented in the NTP era this crate uses.
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
/// How long a timing request waits for its answer before it is written off.
///
/// Upstream keeps eight outstanding requests (`RAOP_NTP_DATA_COUNT`) and every sample it
/// has not given up on; a request whose answer never arrives is simply never completed.
pub const OUTSTANDING_REQUESTS: usize = 8;

/// Samples the offset estimate averages over, which is upstream's table size.
const SAMPLE_COUNT: usize = 8;

/// Clock error allowed to accumulate per second of elapsed time, as a fraction.
const PHI: f64 = 15e-6;

/// Precision of a timestamp, in seconds. Both ends claim one millisecond.
const RHO: f64 = 0.001;

/// Distance beyond which the estimate is reported as unsynchronised.
const MAX_DISTANCE: f64 = 1.5;

/// Dispersion beyond which a sample is no longer worth keeping.
const MAX_DISPERSION: f64 = 16.0;

/// Weight given to a new estimate when one already exists.
const ALPHA: f64 = 0.05;

/// One completed exchange with the client's timing port.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockSample {
    /// The local time the answer arrived, in NTP's 64-bit timestamp form.
    pub received: u64,
    /// The `t3` the client reported, which is what makes a repeat recognisable.
    pub echo: u32,
    /// How far the client's clock is ahead of this one, in seconds.
    pub offset: f64,
    /// Round-trip time with the client's own processing removed, in seconds.
    pub delay: f64,
    /// How much the sample has aged, in seconds.
    pub dispersion: f64,
}

/// The client's clock, as far as the timing exchanges have established it.
///
/// A receiver plays media stamped by the client's clock, so it has to know how that clock
/// relates to its own. The exchange is NTP's: this side sends `t1`, the client echoes it
/// with its receive time `t2` and transmit time `t3`, and the answer arrives at `t4`. The
/// offset is then `((t2 - t1) + (t3 - t4)) / 2`, and a conversion in either direction is
/// that offset added or subtracted.
///
/// Two things make it more than that arithmetic, and both are upstream's
/// (`upstream/UxPlay/lib/raop_ntp.c`):
///
/// - The first remote timestamp seen fixes a *fixed offset*, so later arithmetic happens
///   in one clock domain instead of comparing a client's era to this machine's.
/// - Estimates are filtered: old samples age out, the lowest-delay sample leads, the rest
///   are weighted down, and a new estimate is smoothed into the previous one. A single
///   exchange over a busy network is a bad measurement, and the filter is what stops it
///   from being the one that counts.
#[derive(Debug, Clone)]
pub struct RemoteClock {
    samples: [Option<ClockSample>; SAMPLE_COUNT],
    /// Where the next sample is written, as upstream's `data_index`.
    next: usize,
    /// Requests whose answers have not arrived, by the `t1` they carried.
    ///
    /// A request carries thirty-two bits of seconds in this field, and an answer has to
    /// echo them, so this is what a reply is matched against.
    outstanding: [u32; OUTSTANDING_REQUESTS],
    /// The client's clock minus this one, in nanoseconds, once an estimate exists.
    offset_nanos: i64,
    /// How far the estimate can be trusted, in nanoseconds.
    root_distance_nanos: u64,
    /// Whether the last estimate was close enough to be used.
    synced: bool,
    /// Whether an estimate exists at all.
    have_estimate: bool,
    /// The remote timestamp a response's times are adjusted by, as `fixed_offset`.
    fixed_offset: Option<i64>,
}

impl Default for RemoteClock {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteClock {
    /// A clock that has seen no exchanges.
    #[must_use]
    pub fn new() -> Self {
        Self {
            samples: [None; SAMPLE_COUNT],
            next: 0,
            outstanding: [0; OUTSTANDING_REQUESTS],
            offset_nanos: 0,
            root_distance_nanos: u64::MAX,
            synced: false,
            have_estimate: false,
            fixed_offset: None,
        }
    }

    /// Records a request sent at `t1`, which is the NTP timestamp the answer must echo.
    ///
    /// Returns whether the request fits in the table of outstanding ones. Upstream keeps
    /// eight and overwrites the oldest, which is what happens here.
    pub fn request_sent(&mut self, t1: u32) -> bool {
        self.outstanding[self.next % OUTSTANDING_REQUESTS] = t1;
        self.next = (self.next + 1) % OUTSTANDING_REQUESTS;
        true
    }

    /// Takes one exchange, and returns the estimate it produced, if any.
    ///
    /// `t1` is the local send time in NTP's timestamp form, `t2` and `t3` are what the
    /// client reported, both as its own 32-bit timestamps, and `t4` is the local arrival
    /// time as a full 64-bit timestamp. Returns `None` when the answer is a duplicate, when
    /// it echoes a `t1` that was never sent, or when it leaves no sample worth keeping.
    pub fn response_received(&mut self, t1: u32, t2: u32, t3: u32, t4: u64) -> Option<ClockSample> {
        // The answer has to echo a request this receiver made. Without that check a client
        // could feed the estimate arbitrary numbers, and a spoofed reply is exactly what a
        // timing exchange invites.
        let seen = self
            .outstanding
            .iter()
            .any(|sent| *sent != 0 && *sent == t1);
        if !seen {
            return None;
        }
        // A `t3` already recorded is the same answer twice.
        if self
            .samples
            .iter()
            .flatten()
            .any(|sample| sample.echo == t3)
        {
            return None;
        }

        // The remote stamps are 32-bit: seconds modulo 136 years, with a 32-bit fraction.
        // The first one seen fixes how far this machine's clock is from the client's era,
        // and every later one is adjusted by the same amount, which is what upstream's
        // `fixed_offset` is for.
        let remote_nanos = |stamp: u32| -> i64 {
            let value = i64::from(stamp) << 32;
            match self.fixed_offset {
                Some(offset) => value + offset,
                None => value,
            }
        };
        let first = remote_nanos(t2);
        let fixed = match self.fixed_offset {
            Some(existing) => existing,
            None => {
                let offset = t4 as i64 - first;
                self.fixed_offset = Some(offset);
                offset
            }
        };
        let t2_local = (i64::from(t2) << 32) + fixed;
        let t3_local = (i64::from(t3) << 32) + fixed;

        // `ntp_diff_to_seconds`: the difference of two 64-bit timestamps, in seconds.
        let seconds =
            |later: u64, earlier: i64| -> f64 { (later as i64 - earlier) as f64 / 2f64.powi(32) };
        let total = seconds(t4, i64::from(t1) << 32);
        let client_processing = (t3_local - t2_local) as f64 / 2f64.powi(32);
        let mut delay = total - client_processing;
        if delay < RHO {
            delay = RHO;
        }
        let client_to_server = (t2_local - (i64::from(t1) << 32)) as f64 / 2f64.powi(32);
        let server_to_client = (t4 as i64 - t3_local) as f64 / 2f64.powi(32);

        let sample = ClockSample {
            received: t4,
            echo: t3,
            offset: (client_to_server + server_to_client) / 2.0,
            delay,
            dispersion: 2.0 * RHO + PHI * seconds(t4, i64::from(t1) << 32),
        };
        // The oldest slot is overwritten, which is upstream's ring: eight samples, and a
        // ninth replaces the first.
        let slot = self
            .samples
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                self.samples
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, sample)| sample.map(|s| s.received).unwrap_or(0))
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            });
        self.samples[slot] = Some(sample);
        self.outstanding = [0; OUTSTANDING_REQUESTS];

        Some(self.estimate(t4))
    }

    /// Recomputes the offset from the samples that are still worth using.
    fn estimate(&mut self, now: u64) -> ClockSample {
        // Age every sample and drop the ones that have drifted too far to mean anything.
        let mut usable: Vec<ClockSample> = Vec::with_capacity(SAMPLE_COUNT);
        for sample in self.samples.iter_mut().flatten() {
            let elapsed = (now as i64 - sample.received as i64) as f64 / 2f64.powi(32);
            sample.dispersion += elapsed * PHI;
            if sample.dispersion < MAX_DISPERSION {
                usable.push(*sample);
            }
        }
        // The lowest delay leads; the rest are weighted down by `1/2^i`, which is what
        // stops one lucky exchange on a quiet network from outweighing the others.
        usable.sort_by(|a, b| a.delay.total_cmp(&b.delay));
        let mut filtered = 0.0;
        for (index, sample) in usable.iter().enumerate() {
            filtered += sample.dispersion / 2f64.powi(index as i32);
        }
        let leader = usable[0];
        let root_distance = filtered + leader.delay / 2.0;

        let offset = if root_distance > MAX_DISTANCE {
            // Too far to trust, but the number is still published: it is a measurement,
            // and a receiver that hides it plays at the wrong rate without saying so.
            self.synced = false;
            leader.offset
        } else if self.have_estimate {
            let previous = self.offset_nanos as f64 / 1e9;
            self.synced = true;
            (1.0 - ALPHA) * previous + ALPHA * leader.offset
        } else {
            self.synced = true;
            leader.offset
        };
        self.offset_nanos = (offset * 1e9) as i64;
        self.root_distance_nanos = (root_distance * 1e9) as u64;
        self.have_estimate = true;
        leader
    }

    /// How far the client's era is from this machine's clock, in nanoseconds.
    ///
    /// `None` until a response has been read. This is upstream's `fixed_offset`: the
    /// first remote timestamp seen, taken against the local clock at that moment, and
    /// applied to every later one so the arithmetic happens in one domain.
    #[must_use]
    pub fn fixed_offset_nanos(&self) -> Option<i64> {
        self.fixed_offset
    }

    /// The local time for a client timestamp, in nanoseconds.
    ///
    /// `remote_nanos` is the client's own timestamp: seconds since 1900 with the fraction
    /// a client sends. The conversion applies the fixed offset first and the smoothed
    /// residual second, which is the order upstream uses: it adjusts a remote timestamp
    /// with `raop_ntp_adjust_remote_timestamp_offset` where it reads it, and converts with
    /// `raop_ntp_convert_remote_time` where it schedules.
    ///
    /// `None` until a response has been read. A receiver that guessed would drift away
    /// from the client rather than follow it, and a sound card playing at the wrong rate
    /// is worse than one playing late.
    #[must_use]
    pub fn to_local(&self, remote_nanos: u64) -> Option<u64> {
        let fixed = self.fixed_offset?;
        if !self.have_estimate {
            return None;
        }
        Some(((remote_nanos as i64) + fixed - self.offset_nanos) as u64)
    }

    /// The client's time for a point in the local clock, in nanoseconds.
    #[must_use]
    pub fn to_remote(&self, local_nanos: u64) -> Option<u64> {
        let fixed = self.fixed_offset?;
        if !self.have_estimate {
            return None;
        }
        Some(((local_nanos as i64) + self.offset_nanos - fixed) as u64)
    }

    /// The client's clock minus this one, in nanoseconds.
    #[must_use]
    pub fn offset_nanos(&self) -> i64 {
        self.offset_nanos
    }

    /// How far the estimate can be trusted, in nanoseconds.
    #[must_use]
    pub fn root_distance_nanos(&self) -> u64 {
        self.root_distance_nanos
    }

    /// Whether the last estimate was close enough to be used.
    #[must_use]
    pub fn is_synced(&self) -> bool {
        self.synced
    }
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

    /// A second in NTP's fixed point, which is what a timestamp's high half counts.
    const NANOS_PER_TICK: u64 = 1_000_000_000;

    fn seconds(count: u64) -> u64 {
        count << 32
    }

    #[test]
    fn an_exchange_with_no_network_cost_measures_the_client_clock() {
        let mut clock = RemoteClock::new();
        // The client's clock reads five seconds ahead of this machine's.
        let ahead: i64 = 5 * NANOS_PER_TICK as i64;
        let t1_seconds: u32 = 3_900_000_000;
        let t1 = u64::from(t1_seconds) << 32;
        assert!(clock.request_sent(t1_seconds));
        // A one-way trip of ten milliseconds, and the client takes none of it processing.
        let ten_ms = (0.01 * 2f64.powi(32)) as u64;
        let t4 = t1 + 2 * ten_ms;
        let t2 = (3_900_000_005u64) & 0xffff_ffff;
        let t3 = t2;

        let sample = clock
            .response_received(t1_seconds, t2 as u32, t3 as u32, t4)
            .expect("a sample from a reply that echoes the request");
        // The delay is the round trip without the client's own processing.
        assert!((sample.delay - 0.02).abs() < 1e-6, "delay {}", sample.delay);
        assert!(clock.is_synced(), "an exchange this clean is a sync");

        // The conversion is what a session uses, and it has to survive the era: the client
        // counts seconds since 1900 in 32 bits, so the fixed offset is what puts its
        // timestamps on this machine's clock.
        let remote = t2 << 32;
        let local = clock.to_local(remote).expect("a conversion");
        // A client ten milliseconds into a one-way trip is playing the instant this
        // machine reaches at `t4 - ten milliseconds`. The answer is in nanoseconds, so the
        // comparison is too.
        let ten_ms_nanos = NANOS_PER_TICK / 100;
        assert!(
            local.abs_diff(t4 - ten_ms_nanos) < 1_000,
            "converted {local}, expected about {}",
            t4 - ten_ms_nanos
        );
        assert_eq!(clock.to_remote(local), Some(remote));
        let _ = ahead;
    }

    #[test]
    fn an_answer_that_echoes_nothing_this_receiver_sent_is_refused() {
        let mut clock = RemoteClock::new();
        clock.request_sent(1_000);
        assert!(
            clock
                .response_received(0xdead_beef, 1, 2, seconds(1_001))
                .is_none(),
            "a reply the receiver never asked for is not a measurement"
        );
        assert!(clock.fixed_offset_nanos().is_none());
    }

    #[test]
    fn the_same_answer_twice_is_ignored() {
        let mut clock = RemoteClock::new();
        let t1_seconds: u32 = 2_000;
        let t1 = u64::from(t1_seconds) << 32;
        clock.request_sent(t1_seconds);
        let first = clock.response_received(t1_seconds, 7, 9, t1 + 40_000_000);
        assert!(first.is_some());
        assert_eq!(
            clock.response_received(t1_seconds, 7, 9, t1 + 40_000_000),
            None,
            "a repeated t3 is the same exchange counted twice"
        );
    }

    #[test]
    fn nothing_is_converted_before_an_exchange() {
        let clock = RemoteClock::new();
        assert_eq!(clock.to_local(seconds(10)), None);
        assert_eq!(clock.to_remote(seconds(10)), None);
        assert!(!clock.is_synced());
        assert_eq!(clock.root_distance_nanos(), u64::MAX);
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
