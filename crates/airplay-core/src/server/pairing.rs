// SPDX-License-Identifier: GPL-3.0-or-later
//! Pairing state of a receiver and of one control connection.
//!
//! Upstream keeps the receiver-wide facts in `raop_t` — the PIN, whether a PIN
//! is in use, and the long-term Ed25519 identity of `pairing.c` — and the
//! exchange of one client in `raop_conn_t::session`, next to that connection's
//! FairPlay session (`upstream/UxPlay/lib/raop.c:145-151`, `:627-628`).
//! [`PairingState`] is the shared half and [`PairingSession`] the per-connection
//! half.
//!
//! # Documented differences from upstream
//!
//! Each of these is deliberate, and each has a test:
//!
//! * A `/pair-setup-pin` step 3 is refused unless step 2 verified the client
//!   proof. Upstream derives the step 3 keys from the SRP session key, which is
//!   still the all-zero buffer of a fresh context until the proof is checked
//!   (`upstream/UxPlay/lib/pairing.c:513`, `:587`, `:603`), so two requests
//!   without any PIN let a peer install its own key on the receiver.
//! * `/pair-setup-pin` without a usable PIN is refused. Upstream instead uses
//!   the implicit PIN `"0000"` whenever no PIN is configured
//!   (`upstream/UxPlay/lib/raop.c:627`, `raop_handlers.h:329-332`), which makes
//!   the SRP exchange authenticate nothing.
//! * A one-time PIN is consumed only when pairing succeeds, a repeated
//!   `/pair-pin-start` keeps the PIN that is already pending, and
//!   [`MAX_PIN_FAILURES`] wrong proofs drop it. Upstream clears the PIN on the
//!   first step and regenerates it on every request (`raop_handlers.h:258-267`,
//!   `:329-332`), so any peer can invalidate the PIN a user is reading and a
//!   fixed PIN can be guessed without limit. The cost of consuming late is that
//!   two peers who both know the PIN can each complete a pairing until the first
//!   one succeeds; the failure bound keeps that window small.
//! * A value whose length upstream does not check is validated before it is
//!   used (`raop_handlers.h:366`, `:408-409`), and the steps whose missing state
//!   upstream dereferences as a null pointer (`pairing.c:560`, `:603`) answer
//!   `470` instead.
//! * A `/pair-verify` step 1 that the injected registration predicate refuses
//!   stays refused for the rest of the connection, and only a `/pair-setup-pin`
//!   that proves the PIN lifts it. Upstream stops consulting the list once the
//!   connection is in its setup state (`raop_handlers.h:470-477`), so a client it
//!   has just turned away only has to ask a second time, which would make an
//!   injected list useless as access control.
//! * A key exchange key that yields an all-zero shared secret is refused. A
//!   low-order point would let the peer derive the keystream without holding a
//!   private key, and upstream's OpenSSL derive fails where the port has to
//!   check for itself (`crypto.c:245-265`).
//! * One connection starts one `/pair-verify` key exchange, and one connection
//!   opens at most [`MAX_SETUP_PIN_ATTEMPTS`] SRP contexts. Upstream accepts a
//!   repeat of either (`pairing.c:162-180` only refuses a finished session, and
//!   nothing bounds the modular exponentiation of a setup step), but a repeat
//!   either replaces the keys the connection already agreed on or spends work
//!   for an unauthenticated peer.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::crypto::{self, CryptoError, ed25519_public_key, x25519_public, x25519_shared};
use crate::pairing::{
    ED25519_KEY_SIZE, PAIRING_SIG_SIZE, X25519_KEY_SIZE, decrypt_pair_verify_signature,
    derive_pair_setup_keys, encrypt_pair_verify_signature, pair_setup_gcm_nonces,
    verify_pair_signature,
};
use crate::plist::{self, Plist};
use crate::srp::{
    APPLE_SESSION_KEY_SIZE, APPLE_SRP_HASH_SIZE, APPLE_SRP_SALT_SIZE, AppleSrpServer,
    constant_time_proof_eq,
};

/// Longest client user name upstream accepts (`upstream/UxPlay/lib/pairing.h:24`).
const MAX_USERNAME: usize = 24;

/// Largest SRP public value a client may send: one 2048-bit block.
const MAX_SRP_PUBLIC: usize = crate::srp::APPLE_SRP_MODULUS_SIZE;

/// Length of an AES-GCM authentication tag.
const GCM_TAG_SIZE: usize = 16;

/// Random attempts to draw a PIN before the source is called broken.
const MAX_PIN_ATTEMPTS: usize = 8;

/// SRP contexts one connection may open.
///
/// Every `/pair-setup-pin` step 1 computes a fresh server public value, so an
/// unauthenticated peer could otherwise spend a modular exponentiation per
/// request for as long as it holds a connection slot. Upstream has no such
/// bound; the port bounds the work per connection and relies on the connection
/// limit and the idle deadline for the rest. The value is one above
/// [`MAX_PIN_FAILURES`] so that the per-connection bound never hides the
/// receiver-wide one: the last attempt that can still be counted as a wrong
/// proof is always allowed to happen.
const MAX_SETUP_PIN_ATTEMPTS: usize = MAX_PIN_FAILURES as usize + 1;

/// Wrong proofs a receiver tolerates before it drops a one-time PIN.
///
/// Upstream counts nothing, so a fixed PIN can be guessed online until it is
/// found; this bound is a port decision, and it applies to the shared state so
/// that opening a new connection does not reset it.
pub const MAX_PIN_FAILURES: u32 = 5;

/// Secret bytes a pairing exchange needs.
///
/// Every use of randomness in pairing goes through this trait — the one-time
/// PIN, the SRP salt, the SRP private value and the X25519 ephemeral key — so a
/// test can drive the protocol deterministically. An implementation must be a
/// cryptographically secure generator.
pub trait RandomSource: Send {
    /// Fills `output` completely, or reports that it cannot.
    ///
    /// # Errors
    ///
    /// Returns [`RandomError`] when no secure bytes could be produced.
    fn fill(&mut self, output: &mut [u8]) -> Result<(), RandomError>;
}

/// Why no random bytes could be produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RandomError;

impl fmt::Display for RandomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the random source failed")
    }
}

impl std::error::Error for RandomError {}

/// Random source backed by the operating system.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsRandom;

impl RandomSource for OsRandom {
    fn fill(&mut self, output: &mut [u8]) -> Result<(), RandomError> {
        getrandom::getrandom(output).map_err(|_| RandomError)
    }
}

/// How a receiver treats the PIN of the modern pairing flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinMode {
    /// Pairing with a PIN is refused.
    Disabled,
    /// A PIN `/pair-pin-start` generates, valid until one pairing succeeds.
    OneTime,
    /// A PIN the caller chose. Only the low four digits are used, which is how
    /// upstream stores a configured PIN (`upstream/UxPlay/lib/uxplay.cpp:1654`).
    Fixed(u16),
}

/// Predicate that decides whether a client may start `/pair-verify` without a
/// setup of its own, answering for the client's public key.
pub type ClientRegisteredCheck = Box<dyn Fn(&[u8; ED25519_KEY_SIZE]) -> bool + Send + Sync>;

/// Receiver-wide pairing state, shared by the handlers of every connection.
///
/// A caller creates one value, shares it with the handler factory and keeps it
/// for the lifetime of the receiver: the long-term identity is what a client
/// caches, so a receiver that changes it has to be paired again
/// (`upstream/UxPlay/lib/raop.c:659-674`).
pub struct PairingState {
    identity_seed: [u8; ED25519_KEY_SIZE],
    identity_public: [u8; ED25519_KEY_SIZE],
    pin_mode: PinMode,
    pending_pin: Mutex<Option<u16>>,
    failures: Mutex<u32>,
    random: Mutex<Box<dyn RandomSource>>,
    client_registered: Option<ClientRegisteredCheck>,
}

impl PairingState {
    /// Creates receiver state for `identity_seed` with the given PIN policy,
    /// using the operating system as the random source.
    pub fn new(identity_seed: [u8; ED25519_KEY_SIZE], pin_mode: PinMode) -> Self {
        Self::with_random(identity_seed, pin_mode, Box::new(OsRandom))
    }

    /// Creates receiver state with an explicit random source.
    pub fn with_random(
        identity_seed: [u8; ED25519_KEY_SIZE],
        pin_mode: PinMode,
        random: Box<dyn RandomSource>,
    ) -> Self {
        Self {
            identity_seed,
            identity_public: ed25519_public_key(&identity_seed),
            pin_mode,
            pending_pin: Mutex::new(None),
            failures: Mutex::new(0),
            random: Mutex::new(random),
            client_registered: None,
        }
    }

    /// Adds the predicate that decides whether a client may start
    /// `/pair-verify` on a connection with no setup of its own.
    ///
    /// Upstream asks its application the same question and treats a missing list
    /// as "registered" (`upstream/UxPlay/lib/raop_handlers.h:504-516`), so a
    /// receiver without a predicate accepts every client; injecting one is how a
    /// caller opts into an access-control list. The predicate answers for one
    /// client public key and must not block.
    pub fn with_client_registered(
        mut self,
        check: impl Fn(&[u8; ED25519_KEY_SIZE]) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.client_registered = Some(Box::new(check));
        self
    }

    /// Public key of the receiver's long-term pairing identity.
    ///
    /// This is the `pk` of `/info` and the key every client authenticates the
    /// receiver with, so a caller has to keep it stable across restarts.
    pub fn identity_public(&self) -> [u8; ED25519_KEY_SIZE] {
        self.identity_public
    }

    /// PIN policy of this receiver.
    pub fn pin_mode(&self) -> PinMode {
        self.pin_mode
    }

    /// PIN a user is expected to enter, as four decimal digits.
    ///
    /// A caller shows this to the user after `/pair-pin-start`; it is `None`
    /// while pairing is disabled and while a one-time PIN has not been generated
    /// yet or has been consumed.
    pub fn pending_pin(&self) -> Option<String> {
        let pin = match self.pin_mode {
            PinMode::Disabled => None,
            PinMode::Fixed(pin) => Some(pin % 10_000),
            PinMode::OneTime => *self.guard(&self.pending_pin),
        };
        pin.map(display_pin)
    }

    /// Locks `mutex`, recovering from a poisoned lock.
    ///
    /// A panic in a handler must not turn every later request into a panic, and
    /// none of the values here can be left inconsistent: they are single
    /// integers and options.
    fn guard<'a, T>(&self, mutex: &'a Mutex<T>) -> MutexGuard<'a, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Fills `output` from the random source.
    fn fill_random(&self, output: &mut [u8]) -> Result<(), PairingError> {
        self.guard(&self.random)
            .fill(output)
            .map_err(|_| PairingError::Random)
    }

    /// Draws a PIN in `1..=9999`, the range upstream's `random_pin` produces
    /// (`upstream/UxPlay/lib/pairing.c:484-498`).
    fn random_pin(&self) -> Result<u16, PairingError> {
        let mut random = self.guard(&self.random);
        for _ in 0..MAX_PIN_ATTEMPTS {
            let mut bytes = [0u8; 2];
            random.fill(&mut bytes).map_err(|_| PairingError::Random)?;
            let pin = u16::from_be_bytes(bytes) % 10_000;
            if pin != 0 {
                return Ok(pin);
            }
        }
        // Upstream retries forever when its random source fails; a stated error
        // is the difference between a stuck handler and a bounded one.
        Err(PairingError::Random)
    }

    /// PIN the next setup step has to be answered with.
    fn pin_for_setup(&self) -> Result<[u8; 4], PairingError> {
        match self.pin_mode {
            PinMode::Disabled => Err(PairingError::PinUnavailable),
            PinMode::Fixed(pin) => Ok(digits(pin)),
            PinMode::OneTime => match *self.guard(&self.pending_pin) {
                Some(pin) => Ok(digits(pin)),
                // Upstream would fall back to `"0000"` here; requiring a
                // generated PIN is what makes the exchange authenticate.
                None => Err(PairingError::PinUnavailable),
            },
        }
    }

    /// Starts a PIN exchange and returns the PIN a user has to enter.
    fn start_pin(&self) -> Result<[u8; 4], PairingError> {
        match self.pin_mode {
            PinMode::Disabled => Err(PairingError::PinUnavailable),
            PinMode::Fixed(pin) => Ok(digits(pin)),
            PinMode::OneTime => {
                let mut pending = self.guard(&self.pending_pin);
                let pin = match *pending {
                    // Keep the PIN a user is already looking at; upstream
                    // regenerates it on every request.
                    Some(pin) => pin,
                    None => {
                        let pin = self.random_pin()?;
                        *pending = Some(pin);
                        *self.guard(&self.failures) = 0;
                        pin
                    }
                };
                Ok(digits(pin))
            }
        }
    }

    /// Counts one refused proof, dropping a one-time PIN at the bound.
    fn record_failure(&self) {
        let mut failures = self.guard(&self.failures);
        *failures += 1;
        if *failures >= MAX_PIN_FAILURES {
            *self.guard(&self.pending_pin) = None;
        }
    }

    /// Clears the failure count and consumes a one-time PIN.
    fn record_success(&self) {
        *self.guard(&self.failures) = 0;
        *self.guard(&self.pending_pin) = None;
    }
}

impl fmt::Debug for PairingState {
    /// Describes the receiver without printing its private seed.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingState")
            .field("identity_public", &hex(&self.identity_public))
            .field("pin_mode", &self.pin_mode)
            .finish_non_exhaustive()
    }
}

impl Drop for PairingState {
    /// Overwrites the identity seed when the receiver goes away.
    ///
    /// Best effort by design: it clears the copy this value owns, not whatever a
    /// crashing dump or a swapped-out page may still hold.
    fn drop(&mut self) {
        self.identity_seed.fill(0);
    }
}

/// Four zero-padded decimal digits, `snprintf("%04u")` of the C code.
fn digits(pin: u16) -> [u8; 4] {
    let pin = pin % 10_000;
    [
        b'0' + (pin / 1000) as u8,
        b'0' + (pin / 100 % 10) as u8,
        b'0' + (pin / 10 % 10) as u8,
        b'0' + (pin % 10) as u8,
    ]
}

/// Four zero-padded decimal digits as text.
fn display_pin(pin: u16) -> String {
    digits(pin).iter().map(|byte| *byte as char).collect()
}

/// Lower-case hexadecimal, for diagnostics that must not carry raw key bytes.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Why a pairing request was refused.
///
/// The control service answers every variant with `470 Client Authentication
/// Failure` and no body, the status upstream uses for a failed pairing step
/// (`upstream/UxPlay/lib/raop_handlers.h:431-432`). No variant carries key
/// material, so an error can be logged.
#[derive(Debug)]
pub enum PairingError {
    /// The request body was not a binary property list.
    NotPlist,
    /// The body carried neither the keys of a known step nor their types.
    UnexpectedKeys,
    /// A value had a length this port does not accept.
    InvalidLength(&'static str),
    /// A step arrived before the step it depends on.
    OutOfOrder(&'static str),
    /// Pairing with a PIN is disabled, or no PIN is pending.
    PinUnavailable,
    /// The client proof did not match the PIN, or a signature did not verify.
    AuthenticationFailed,
    /// The random source could not produce bytes.
    Random,
    /// The SRP exchange was refused.
    Srp(crate::srp::SrpError),
    /// A cryptographic primitive refused its input.
    Crypto(CryptoError),
}

impl PairingError {
    /// Status code upstream answers a failed pairing step with.
    pub fn status(&self) -> u16 {
        470
    }

    /// Reason phrase that belongs to [`PairingError::status`].
    pub fn reason(&self) -> &'static str {
        "Client Authentication Failure"
    }
}

impl fmt::Display for PairingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPlist => formatter.write_str("the request body is not a binary property list"),
            Self::UnexpectedKeys => {
                formatter.write_str("the request carries no known pairing step")
            }
            Self::InvalidLength(what) => write!(formatter, "invalid length of {what}"),
            Self::OutOfOrder(step) => {
                write!(formatter, "{step} has not completed on this connection")
            }
            Self::PinUnavailable => formatter.write_str("no PIN is available for pairing"),
            Self::AuthenticationFailed => formatter.write_str("client authentication failed"),
            Self::Random => formatter.write_str("the random source failed"),
            Self::Srp(error) => write!(formatter, "SRP exchange refused: {error}"),
            Self::Crypto(error) => write!(formatter, "cryptographic primitive refused: {error}"),
        }
    }
}

impl std::error::Error for PairingError {}

/// What the control service answers a pairing request with.
pub(crate) enum PairingReply {
    /// Add nothing to the default `200 OK`: the answer upstream gives when a
    /// step has nothing to say, for example when a client should fall back to
    /// the legacy setup (`upstream/UxPlay/lib/raop_handlers.h:319-324`, `:447-450`,
    /// `:474-476`).
    Silence,
    /// Binary property list body.
    Plist(plist::Value),
    /// Opaque body with `Content-Type: application/octet-stream`.
    Octets(Vec<u8>),
    /// `200 OK` with nothing added, after which the connection is closed
    /// (`upstream/UxPlay/lib/raop_handlers.h:533-537`).
    Disconnect,
}

/// What one connection established before it asked for a media session.
///
/// The levels are evidence, not permissions: even [`SessionAuth::Verified`] only
/// proves that the peer holds the keys it presented, so a caller that needs an
/// access decision asks its own registry (see
/// [`PairingState::with_client_registered`]) or the password gate of `SETUP`.
// The SETUP handler is the only production consumer and lands in the next step
// of W5c; this allow goes away with it (the tests already exercise both items).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionAuth {
    /// The connection authenticated nothing.
    None,
    /// `/pair-setup` completed; that flow authenticates neither side
    /// (`upstream/UxPlay/lib/raop_handlers.h:440-460`).
    LegacySetup,
    /// `/pair-setup-pin` step 3 installed a client key after the PIN proof.
    PinVerified,
    /// The client signed the key exchange, so `/pair-verify` finished here.
    Verified,
}

/// Key material of a connection that finished `/pair-verify`.
///
/// A media session derives its stream keys from these values, so they are only
/// produced for a connection that completed the exchange, and only once
/// ([`PairingSession::take_session_keys`]).
#[allow(dead_code)]
pub(crate) struct SessionKeys {
    /// X25519 shared secret of the connection's key exchange.
    pub(crate) ecdh_secret: [u8; X25519_KEY_SIZE],
    /// Public key the client presented in the verified exchange.
    ///
    /// It is *claimed* rather than proven: the signature only shows that the
    /// peer holds the matching private key. Whether that client may be served is
    /// the caller's registration decision.
    pub(crate) client_identity: Option<[u8; ED25519_KEY_SIZE]>,
}

/// Lifecycle of one connection's pairing exchange, the port of the status values
/// of `upstream/UxPlay/lib/pairing.c:148-159`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Nothing is set up on this connection.
    Initial,
    /// A setup step completed, so `/pair-verify` may start here.
    Setup,
    /// The first `/pair-verify` step completed.
    Handshake,
    /// `/pair-verify` verified the client on this connection.
    Finished,
}

impl Status {
    /// Whether `/pair-verify` may start, the inverse of upstream's
    /// `pairing_session_check_handshake_status`.
    fn allows_handshake(self) -> bool {
        matches!(self, Self::Setup | Self::Handshake)
    }
}

/// Pairing exchange of one connection, upstream's `raop_conn_t::session`.
///
/// Temporary material — the SRP context, the session key of a verified proof,
/// the X25519 key pair and the key the client presented — belongs to the client
/// that opened the connection and is dropped with it.
pub(crate) struct PairingSession {
    shared: Arc<PairingState>,
    status: Status,
    srp: Option<AppleSrpServer>,
    session_key: Option<[u8; APPLE_SESSION_KEY_SIZE]>,
    client_public_key: Option<[u8; ED25519_KEY_SIZE]>,
    ecdh_public: Option<[u8; X25519_KEY_SIZE]>,
    ecdh_client_public: Option<[u8; X25519_KEY_SIZE]>,
    ecdh_shared: Option<[u8; X25519_KEY_SIZE]>,
    registration_refused: bool,
    setup_pin_attempts: usize,
    pin_verified: bool,
}

impl PairingSession {
    /// Starts a session on the shared receiver state.
    pub(crate) fn new(shared: Arc<PairingState>) -> Self {
        Self {
            shared,
            status: Status::Initial,
            srp: None,
            session_key: None,
            client_public_key: None,
            ecdh_public: None,
            ecdh_client_public: None,
            ecdh_shared: None,
            registration_refused: false,
            setup_pin_attempts: 0,
            pin_verified: false,
        }
    }

    /// `/pair-pin-start`: `raop_handler_pairpinstart`.
    ///
    /// Upstream adds nothing to the reply (`upstream/UxPlay/lib/raop_handlers.h:270-273`);
    /// the PIN reaches the user through [`PairingState::pending_pin`], which
    /// keeps the display out of the handler and out of any held lock.
    pub(crate) fn pin_start(&mut self) -> Result<PairingReply, PairingError> {
        self.shared.start_pin()?;
        Ok(PairingReply::Silence)
    }

    /// What this connection established, for a caller that has to branch on it.
    #[allow(dead_code)]
    pub(crate) fn authentication(&self) -> SessionAuth {
        if self.status == Status::Finished {
            return SessionAuth::Verified;
        }
        if self.pin_verified {
            return SessionAuth::PinVerified;
        }
        if matches!(self.status, Status::Setup | Status::Handshake) {
            return SessionAuth::LegacySetup;
        }
        SessionAuth::None
    }

    /// Takes the key material of a connection that finished `/pair-verify`.
    ///
    /// The material is handed out at most once, and only after the second
    /// `/pair-verify` step verified the client's signature
    /// (`upstream/UxPlay/lib/pairing.c:108-119` hands it out for any connection
    /// that started a handshake, which this port deliberately narrows). A caller
    /// that gets `None` has no verified session on this connection and must not
    /// derive stream keys.
    #[allow(dead_code)]
    pub(crate) fn take_session_keys(&mut self) -> Option<SessionKeys> {
        if self.status != Status::Finished {
            return None;
        }
        let ecdh_secret = self.ecdh_shared.take()?;
        Some(SessionKeys {
            ecdh_secret,
            client_identity: self.client_public_key,
        })
    }

    /// `/pair-setup-pin`: `raop_handler_pairsetup_pin`.
    ///
    /// The three steps are told apart by the keys they carry, in the order
    /// upstream tests them (`upstream/UxPlay/lib/raop_handlers.h:310-430`).
    pub(crate) fn setup_pin(&mut self, body: &[u8]) -> Result<PairingReply, PairingError> {
        // Upstream parses with `plist_from_bin` (`raop_handlers.h:302`), so an
        // XML property list is refused even though the plist layer could read it.
        if !body.starts_with(b"bplist00") {
            return Err(PairingError::NotPlist);
        }
        let request = Plist::parse(body).map_err(|_| PairingError::NotPlist)?;
        let method = request.get("method").and_then(plist::Value::as_string);
        let user = request.get("user").and_then(plist::Value::as_string);
        if let (Some(method), Some(user)) = (method, user) {
            return self.setup_pin_one(method, user);
        }
        let public_key = request.get("pk").and_then(plist::Value::as_data);
        let proof = request.get("proof").and_then(plist::Value::as_data);
        if let (Some(public_key), Some(proof)) = (public_key, proof) {
            return self.setup_pin_two(public_key, proof);
        }
        let epk = request.get("epk").and_then(plist::Value::as_data);
        let auth_tag = request.get("authTag").and_then(plist::Value::as_data);
        if let (Some(epk), Some(auth_tag)) = (epk, auth_tag) {
            return self.setup_pin_three(epk, auth_tag);
        }
        Err(PairingError::UnexpectedKeys)
    }

    /// First step: try a PIN, answer with the SRP public value and salt.
    fn setup_pin_one(&mut self, method: &str, user: &str) -> Result<PairingReply, PairingError> {
        // Upstream compares with `strncmp(method, "pin", strlen(method))`, so any
        // prefix of "pin" is accepted and anything else falls through to the
        // default `200 OK` with no body (`raop_handlers.h:319-324`).
        if !"pin".starts_with(method) {
            return Ok(PairingReply::Silence);
        }
        let user = user.as_bytes();
        if user.is_empty() || user.len() > MAX_USERNAME {
            return Err(PairingError::InvalidLength("pairing user name"));
        }
        if self.setup_pin_attempts >= MAX_SETUP_PIN_ATTEMPTS {
            // Each attempt costs this receiver a modular exponentiation, and
            // nothing in the request is authenticated yet.
            return Err(PairingError::AuthenticationFailed);
        }
        self.setup_pin_attempts += 1;
        let pin = self.shared.pin_for_setup()?;
        let mut salt = [0u8; APPLE_SRP_SALT_SIZE];
        self.shared.fill_random(&mut salt)?;
        let mut private_b = [0u8; 32];
        self.shared.fill_random(&mut private_b)?;
        let server =
            AppleSrpServer::new(user, &pin, &salt, &private_b).map_err(PairingError::Srp)?;
        // The client's public value is not known yet, so the answer needs the
        // server public value on its own (`pairing.c:543-550`).
        let public = server.server_public().map_err(PairingError::Srp)?;
        self.srp = Some(server);
        self.session_key = None;
        self.client_public_key = None;
        Ok(PairingReply::Plist(plist::dictionary([
            ("pk", plist::Value::Data(public)),
            ("salt", plist::Value::Data(salt.to_vec())),
        ])))
    }

    /// Second step: check the client proof and answer with the server proof.
    fn setup_pin_two(
        &mut self,
        public_key: &[u8],
        proof: &[u8],
    ) -> Result<PairingReply, PairingError> {
        let Some(server) = self.srp.as_ref() else {
            return Err(PairingError::OutOfOrder("pair-setup-pin step 1"));
        };
        if public_key.is_empty() || public_key.len() > MAX_SRP_PUBLIC {
            return Err(PairingError::InvalidLength("SRP client public key"));
        }
        if proof.len() != APPLE_SRP_HASH_SIZE {
            return Err(PairingError::InvalidLength("SRP client proof"));
        }
        let response = match server.respond(public_key) {
            Ok(response) => response,
            Err(error) => {
                self.srp = None;
                self.shared.record_failure();
                return Err(PairingError::Srp(error));
            }
        };
        // The SRP context is spent either way: upstream keeps its own until the
        // next step and then frees it, so a retry of this step crashes it.
        self.srp = None;
        if !constant_time_proof_eq(&response.expected_proof, proof) {
            self.shared.record_failure();
            return Err(PairingError::AuthenticationFailed);
        }
        self.session_key = Some(response.session_key);
        Ok(PairingReply::Plist(plist::dictionary([(
            "proof",
            plist::Value::Data(response.server_proof.to_vec()),
        )])))
    }

    /// Third step: exchange the long-term public keys.
    fn setup_pin_three(
        &mut self,
        epk: &[u8],
        auth_tag: &[u8],
    ) -> Result<PairingReply, PairingError> {
        // The session key only exists once step 2 verified the proof; upstream
        // derives the keys below from the zero-filled key of an unverified
        // context, which lets a peer skip this check entirely.
        let Some(session_key) = self.session_key else {
            return Err(PairingError::OutOfOrder("pair-setup-pin step 2"));
        };
        if epk.len() != ED25519_KEY_SIZE {
            return Err(PairingError::InvalidLength("pairing key exchange key"));
        }
        if auth_tag.len() != GCM_TAG_SIZE {
            return Err(PairingError::InvalidLength("pairing authentication tag"));
        }
        let keys = derive_pair_setup_keys(&session_key);
        let (decrypt_iv, encrypt_iv) = pair_setup_gcm_nonces(&session_key);
        let client_public =
            match crypto::aes128_gcm16_decrypt(&keys.key, &decrypt_iv, epk, auth_tag) {
                Ok(plaintext) if plaintext.len() == ED25519_KEY_SIZE => plaintext,
                Ok(_) => return Err(PairingError::InvalidLength("pairing key exchange key")),
                Err(error) => {
                    // The session key is spent; another attempt starts over.
                    self.session_key = None;
                    return Err(PairingError::Crypto(error));
                }
            };
        let identity = self.shared.identity_public();
        let (sealed, tag) = crypto::aes128_gcm16_encrypt(&keys.key, &encrypt_iv, &identity)
            .map_err(PairingError::Crypto)?;
        self.client_public_key = client_public.try_into().ok();
        self.session_key = None;
        self.status = Status::Setup;
        // The client proved the PIN, which is the strongest evidence this
        // connection can carry until `/pair-verify` finishes.
        self.pin_verified = true;
        // A client that just proved it knows the PIN is no longer a refused one,
        // which is what keeps a legitimate pairing able to verify afterwards.
        self.registration_refused = false;
        self.shared.record_success();
        Ok(PairingReply::Plist(plist::dictionary([
            ("epk", plist::Value::Data(sealed)),
            ("authTag", plist::Value::Data(tag.to_vec())),
        ])))
    }

    /// `/pair-setup`: `raop_handler_pairsetup`.
    ///
    /// The legacy flow carries no secret: upstream ignores the content of the
    /// 32-byte body and answers with the receiver's public key
    /// (`upstream/UxPlay/lib/raop_handlers.h:440-460`), which is why the reply
    /// must not be treated as authentication anywhere.
    pub(crate) fn setup(&mut self, body: &[u8]) -> Result<PairingReply, PairingError> {
        if body.len() != ED25519_KEY_SIZE {
            return Ok(PairingReply::Silence);
        }
        self.status = Status::Setup;
        Ok(PairingReply::Octets(self.shared.identity_public().to_vec()))
    }

    /// `/pair-verify`: `raop_handler_pairverify`.
    pub(crate) fn verify(&mut self, body: &[u8]) -> Result<PairingReply, PairingError> {
        let mut register_check = false;
        if !self.status.allows_handshake() {
            if self.shared.pin_mode() == PinMode::Disabled {
                // The default reply without a body is the client's cue to fall
                // back to `/pair-setup` (`raop_handlers.h:474-476`).
                return Ok(PairingReply::Silence);
            }
            self.status = Status::Setup;
            register_check = true;
        } else if self.registration_refused {
            // Upstream stops checking the client as soon as the connection is in
            // its setup state (`:470-477`), so a client it just refused only has
            // to ask again. A refusal therefore stays for the rest of this
            // connection, which is what makes an injected registration list an
            // access control rather than a single-shot gate; a client that
            // authenticates with the PIN clears it below.
            register_check = true;
        }
        if body.len() < 4 {
            return Ok(PairingReply::Silence);
        }
        match body[0] {
            1 => {
                if self.status != Status::Setup {
                    // One key exchange per connection: a second one would quietly
                    // replace the keys the first one agreed on, which is what the
                    // connection's session material is derived from.
                    return Ok(PairingReply::Silence);
                }
                self.verify_handshake(body, register_check)
            }
            0 => self.verify_finish(body),
            _ => Ok(PairingReply::Silence),
        }
    }

    /// First `/pair-verify` step: start the key exchange and sign our key.
    fn verify_handshake(
        &mut self,
        body: &[u8],
        register_check: bool,
    ) -> Result<PairingReply, PairingError> {
        if body.len() != 4 + X25519_KEY_SIZE + ED25519_KEY_SIZE {
            return Ok(PairingReply::Silence);
        }
        let client_ecdh = array::<X25519_KEY_SIZE>(&body[4..36], "pair-verify key exchange key")?;
        let client_identity = array::<ED25519_KEY_SIZE>(&body[36..68], "pair-verify client key")?;
        if register_check {
            if let Some(check) = self.shared.client_registered.as_ref() {
                // Without a predicate upstream treats every client as registered
                // (`raop_handlers.h:504-516`); a caller that needs the check
                // injects one.
                if !check(&client_identity) {
                    self.registration_refused = true;
                    return Ok(PairingReply::Silence);
                }
            }
        }
        let mut private = [0u8; X25519_KEY_SIZE];
        self.shared.fill_random(&mut private)?;
        let public = x25519_public(&private);
        let shared = x25519_shared(&private, &client_ecdh);
        if shared.iter().all(|byte| *byte == 0) {
            // A low-order public key yields a zero secret, which would let the
            // peer derive the keystream without holding a private key.
            return Err(PairingError::AuthenticationFailed);
        }
        let signature = encrypt_pair_verify_signature(
            &self.shared.identity_seed,
            &public,
            &client_ecdh,
            &shared,
        )
        .map_err(PairingError::Crypto)?;
        self.ecdh_public = Some(public);
        self.ecdh_client_public = Some(client_ecdh);
        self.ecdh_shared = Some(shared);
        self.client_public_key = Some(client_identity);
        self.status = Status::Handshake;
        let mut reply = public.to_vec();
        reply.extend_from_slice(&signature);
        Ok(PairingReply::Octets(reply))
    }

    /// Second `/pair-verify` step: verify the client's signature.
    ///
    /// A signature that does not verify is answered with the default `200 OK`
    /// and closes the connection, which is what upstream asks its httpd to do
    /// (`upstream/UxPlay/lib/raop_handlers.h:533-537`); that reply carries no
    /// content type, unlike the successful one below it.
    fn verify_finish(&mut self, body: &[u8]) -> Result<PairingReply, PairingError> {
        if body.len() != 4 + PAIRING_SIG_SIZE {
            return Ok(PairingReply::Silence);
        }
        let (Some(shared), Some(our_ecdh), Some(client_ecdh), Some(client_identity)) = (
            self.ecdh_shared,
            self.ecdh_public,
            self.ecdh_client_public,
            self.client_public_key,
        ) else {
            return Ok(PairingReply::Disconnect);
        };
        if self.status != Status::Handshake {
            return Ok(PairingReply::Disconnect);
        }
        let encrypted = array::<PAIRING_SIG_SIZE>(&body[4..], "pair-verify signature")?;
        let Ok(signature) = decrypt_pair_verify_signature(&encrypted, &shared) else {
            return Ok(PairingReply::Disconnect);
        };
        if !verify_pair_signature(&signature, &client_ecdh, &our_ecdh, &client_identity) {
            return Ok(PairingReply::Disconnect);
        }
        self.status = Status::Finished;
        Ok(PairingReply::Octets(Vec::new()))
    }
}

impl fmt::Debug for PairingSession {
    /// Describes the exchange without printing key material.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingSession")
            .field("status", &self.status)
            .field("srp", &self.srp.is_some())
            .field("session_key", &self.session_key.is_some())
            .finish_non_exhaustive()
    }
}

impl Drop for PairingSession {
    /// Overwrites the secrets this connection agreed on.
    ///
    /// The same best-effort caveat as [`PairingState`] applies; the point is that
    /// a closed connection does not leave its session key or shared secret in a
    /// buffer that a later connection could reuse.
    fn drop(&mut self) {
        if let Some(key) = self.session_key.as_mut() {
            key.fill(0);
        }
        if let Some(secret) = self.ecdh_shared.as_mut() {
            secret.fill(0);
        }
    }
}

/// Fixed-size array from a slice of external input.
fn array<const N: usize>(slice: &[u8], what: &'static str) -> Result<[u8; N], PairingError> {
    slice
        .try_into()
        .map_err(|_| PairingError::InvalidLength(what))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{Aes128CtrState, ed25519_sign, ed25519_verify};
    use crate::pairing::derive_pair_verify_keys;
    use crate::srp::test_client;

    /// Identity seed of the receiver under test.
    const IDENTITY_SEED: [u8; 32] = [0x5a; 32];

    /// Deterministic random source: it repeats a fixed byte sequence, so every
    /// value a test sees is reproducible.
    struct FixedRandom {
        next: u8,
    }

    impl FixedRandom {
        fn new() -> Self {
            Self { next: 1 }
        }
    }

    impl RandomSource for FixedRandom {
        fn fill(&mut self, output: &mut [u8]) -> Result<(), RandomError> {
            for byte in output.iter_mut() {
                *byte = self.next;
                self.next = self.next.wrapping_add(1);
            }
            Ok(())
        }
    }

    fn state(pin_mode: PinMode) -> Arc<PairingState> {
        Arc::new(PairingState::with_random(
            IDENTITY_SEED,
            pin_mode,
            Box::new(FixedRandom::new()),
        ))
    }

    /// Body of one `/pair-setup-pin` step as a binary property list.
    fn request(entries: Vec<(&str, plist::Value)>) -> Vec<u8> {
        Plist::from_value(plist::dictionary(entries))
            .encode_binary()
            .expect("encode")
    }

    fn body_of(reply: &PairingReply) -> &[u8] {
        match reply {
            PairingReply::Octets(body) => body,
            _ => panic!("expected an opaque body"),
        }
    }

    fn plist_of(reply: &PairingReply) -> Plist {
        match reply {
            PairingReply::Plist(value) => Plist::from_value(value.clone()),
            _ => panic!("expected a property list"),
        }
    }

    fn field(plist: &Plist, key: &'static str) -> Vec<u8> {
        plist.get_data(key).expect("data field").to_vec()
    }

    /// Body of the first `/pair-setup-pin` step for `username`.
    fn step_one_request(username: &[u8]) -> Vec<u8> {
        request(vec![
            ("method", plist::Value::from("pin")),
            (
                "user",
                plist::Value::from(String::from_utf8_lossy(username).into_owned()),
            ),
        ])
    }

    /// Runs steps 1 and 2 of `/pair-setup-pin` and returns the client exchange.
    fn setup_pin_two_steps(
        session: &mut PairingSession,
        pin: &[u8],
        username: &[u8],
    ) -> (test_client::Exchange, [u8; 16]) {
        let first = session
            .setup_pin(&request(vec![
                ("method", plist::Value::from("pin")),
                (
                    "user",
                    plist::Value::from(String::from_utf8_lossy(username).into_owned()),
                ),
            ]))
            .expect("step 1");
        let reply = plist_of(&first);
        let salt: [u8; 16] = field(&reply, "salt").try_into().expect("salt");
        let server_public = field(&reply, "pk");
        let exchange = test_client::exchange(username, pin, &salt, &[0x07; 32], &server_public);
        let second = session
            .setup_pin(&request(vec![
                ("pk", plist::Value::Data(exchange.public_key.clone())),
                ("proof", plist::Value::Data(exchange.proof.to_vec())),
            ]))
            .expect("step 2");
        assert_eq!(
            field(&plist_of(&second), "proof"),
            exchange.server_proof.to_vec()
        );
        (exchange, salt)
    }

    /// Seals `plaintext` the way a client seals its key exchange key in step 3.
    fn seal_step_three(
        session_key: &[u8; APPLE_SESSION_KEY_SIZE],
        plaintext: &[u8],
    ) -> (Vec<u8>, Vec<u8>) {
        let keys = derive_pair_setup_keys(session_key);
        let (decrypt_iv, _) = pair_setup_gcm_nonces(session_key);
        let (sealed, tag) =
            crypto::aes128_gcm16_encrypt(&keys.key, &decrypt_iv, plaintext).expect("seal");
        (sealed, tag.to_vec())
    }

    fn step_three_request(epk: Vec<u8>, auth_tag: Vec<u8>) -> Vec<u8> {
        request(vec![
            ("epk", plist::Value::Data(epk)),
            ("authTag", plist::Value::Data(auth_tag)),
        ])
    }

    #[test]
    fn pin_start_reports_the_pin_a_user_has_to_enter() {
        let shared = state(PinMode::OneTime);
        let mut session = PairingSession::new(Arc::clone(&shared));
        assert!(session.pin_start().is_ok());
        let pin = shared.pending_pin().expect("a PIN is pending");
        assert_eq!(pin.len(), 4);
        assert!(pin.chars().all(|digit| digit.is_ascii_digit()));
        // A second request keeps the PIN a user may already be reading.
        assert!(session.pin_start().is_ok());
        assert_eq!(shared.pending_pin().as_deref(), Some(pin.as_str()));

        let fixed = state(PinMode::Fixed(42));
        assert_eq!(fixed.pending_pin().as_deref(), Some("0042"));
        assert!(PairingSession::new(fixed).pin_start().is_ok());

        let disabled = state(PinMode::Disabled);
        assert!(matches!(
            PairingSession::new(disabled).pin_start(),
            Err(PairingError::PinUnavailable)
        ));
    }

    #[test]
    fn setup_without_a_pin_is_refused() {
        let mut session = PairingSession::new(state(PinMode::Disabled));
        assert!(matches!(
            session.setup_pin(&request(vec![
                ("method", plist::Value::from("pin")),
                ("user", plist::Value::from("client")),
            ])),
            Err(PairingError::PinUnavailable)
        ));

        // A one-time PIN only exists after `/pair-pin-start`.
        let mut session = PairingSession::new(state(PinMode::OneTime));
        assert!(matches!(
            session.setup_pin(&request(vec![
                ("method", plist::Value::from("pin")),
                ("user", plist::Value::from("client")),
            ])),
            Err(PairingError::PinUnavailable)
        ));
    }

    #[test]
    fn setup_pin_requires_a_binary_plist_and_a_known_step() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(shared);
        assert!(matches!(
            session.setup_pin(b"<?xml version=\"1.0\"?><plist/>"),
            Err(PairingError::NotPlist)
        ));
        assert!(matches!(
            session.setup_pin(&request(vec![("method", plist::Value::from("pin"))])),
            Err(PairingError::UnexpectedKeys)
        ));
        // Upstream's `strncmp` accepts any prefix of "pin" and falls back to the
        // default reply for anything else.
        assert!(matches!(
            session.setup_pin(&request(vec![
                ("method", plist::Value::from("pins")),
                ("user", plist::Value::from("client")),
            ])),
            Ok(PairingReply::Silence)
        ));
    }

    #[test]
    fn setup_pin_rejects_out_of_order_and_oversized_steps() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));

        // Step 2 and step 3 before step 1: upstream dereferences a null SRP
        // context here, the port answers 470 through this error.
        assert!(matches!(
            session.setup_pin(&request(vec![
                ("pk", plist::Value::Data(vec![2; 32])),
                ("proof", plist::Value::Data(vec![0; 20])),
            ])),
            Err(PairingError::OutOfOrder(_))
        ));
        assert!(matches!(
            session.setup_pin(&step_three_request(vec![0; 32], vec![0; 16])),
            Err(PairingError::OutOfOrder(_))
        ));

        assert!(session.pin_start().is_ok());
        let username = b"AA:BB:CC:DD:EE:FF";
        let long_user = request(vec![
            ("method", plist::Value::from("pin")),
            ("user", plist::Value::from("x".repeat(25))),
        ]);
        assert!(matches!(
            session.setup_pin(&long_user),
            Err(PairingError::InvalidLength("pairing user name"))
        ));
        let (exchange, _) = setup_pin_two_steps(&mut session, b"1234", username);
        // Step 2 already consumed the SRP context, so it cannot run twice.
        assert!(matches!(
            session.setup_pin(&request(vec![
                ("pk", plist::Value::Data(exchange.public_key.clone())),
                ("proof", plist::Value::Data(exchange.proof.to_vec())),
            ])),
            Err(PairingError::OutOfOrder(_))
        ));

        // A public value or proof of the wrong length is refused before it
        // reaches the big-integer math, where upstream over-reads the buffer.
        let mut fresh = PairingSession::new(Arc::clone(&shared));
        fresh
            .setup_pin(&request(vec![
                ("method", plist::Value::from("pin")),
                ("user", plist::Value::from("client")),
            ]))
            .expect("step 1");
        let oversized_public = request(vec![
            ("pk", plist::Value::Data(vec![2; MAX_SRP_PUBLIC + 1])),
            ("proof", plist::Value::Data(vec![0; APPLE_SRP_HASH_SIZE])),
        ]);
        assert!(matches!(
            fresh.setup_pin(&oversized_public),
            Err(PairingError::InvalidLength("SRP client public key"))
        ));
        let short_proof = request(vec![
            ("pk", plist::Value::Data(vec![2; 32])),
            (
                "proof",
                plist::Value::Data(vec![0; APPLE_SRP_HASH_SIZE - 1]),
            ),
        ]);
        assert!(matches!(
            fresh.setup_pin(&short_proof),
            Err(PairingError::InvalidLength("SRP client proof"))
        ));
    }

    #[test]
    fn setup_pin_refuses_the_third_step_without_a_verified_proof() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));
        let username = b"AA:BB:CC:DD:EE:FF";
        let first = session
            .setup_pin(&request(vec![
                ("method", plist::Value::from("pin")),
                ("user", plist::Value::from("AA:BB:CC:DD:EE:FF")),
            ]))
            .expect("step 1");

        // The attack upstream is open to: derive the step 3 keys from the
        // all-zero session key of a context whose proof was never checked.
        let zero_key = [0u8; APPLE_SESSION_KEY_SIZE];
        let attacker_key = [0x11u8; ED25519_KEY_SIZE];
        let (epk, auth_tag) = seal_step_three(&zero_key, &attacker_key);
        let outcome = session.setup_pin(&step_three_request(epk, auth_tag));
        assert!(
            matches!(outcome, Err(PairingError::OutOfOrder(_))),
            "step 3 must not run without a verified proof"
        );
        assert!(session.client_public_key.is_none());
        assert_eq!(session.status, Status::Initial);

        // The same connection can still pair properly afterwards.
        let salt: [u8; 16] = field(&plist_of(&first), "salt").try_into().expect("salt");
        let server_public = field(&plist_of(&first), "pk");
        let exchange = test_client::exchange(username, b"1234", &salt, &[0x07; 32], &server_public);
        session
            .setup_pin(&request(vec![
                ("pk", plist::Value::Data(exchange.public_key.clone())),
                ("proof", plist::Value::Data(exchange.proof.to_vec())),
            ]))
            .expect("step 2");
        let (epk, auth_tag) = seal_step_three(&exchange.session_key, &attacker_key);
        let reply = session
            .setup_pin(&step_three_request(epk, auth_tag))
            .expect("step 3");
        assert_eq!(session.client_public_key, Some(attacker_key));
        assert_eq!(session.status, Status::Setup);
        let reply = plist_of(&reply);
        let keys = derive_pair_setup_keys(&exchange.session_key);
        let (_, encrypt_iv) = pair_setup_gcm_nonces(&exchange.session_key);
        let identity = crypto::aes128_gcm16_decrypt(
            &keys.key,
            &encrypt_iv,
            &field(&reply, "epk"),
            &field(&reply, "authTag"),
        )
        .expect("the reply opens with the second nonce");
        assert_eq!(identity, shared.identity_public().to_vec());
    }

    #[test]
    fn setup_pin_refuses_a_wrong_proof_and_bounds_attempts() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));
        let username = b"AA:BB:CC:DD:EE:FF";
        let first = session
            .setup_pin(&request(vec![
                ("method", plist::Value::from("pin")),
                ("user", plist::Value::from("AA:BB:CC:DD:EE:FF")),
            ]))
            .expect("step 1");
        let salt: [u8; 16] = field(&plist_of(&first), "salt").try_into().expect("salt");
        let server_public = field(&plist_of(&first), "pk");
        // The client used the wrong PIN, so its proof cannot match.
        let exchange = test_client::exchange(username, b"9999", &salt, &[0x07; 32], &server_public);
        assert!(matches!(
            session.setup_pin(&request(vec![
                ("pk", plist::Value::Data(exchange.public_key)),
                ("proof", plist::Value::Data(exchange.proof.to_vec())),
            ])),
            Err(PairingError::AuthenticationFailed)
        ));
        // A wrong proof is refused, and the session needs a new step 1.
        assert!(matches!(
            session.setup_pin(&request(vec![
                ("pk", plist::Value::Data(vec![2; 32])),
                ("proof", plist::Value::Data(vec![0; 20])),
            ])),
            Err(PairingError::OutOfOrder(_))
        ));
    }

    #[test]
    fn a_one_time_pin_survives_a_bounded_number_of_failures() {
        let shared = state(PinMode::OneTime);
        let mut session = PairingSession::new(Arc::clone(&shared));
        session.pin_start().expect("pin start");
        let pin = shared.pending_pin().expect("pending PIN");
        let username = b"AA:BB:CC:DD:EE:FF";
        for attempt in 0..MAX_PIN_FAILURES {
            let first = session
                .setup_pin(&request(vec![
                    ("method", plist::Value::from("pin")),
                    ("user", plist::Value::from("AA:BB:CC:DD:EE:FF")),
                ]))
                .expect("step 1");
            let salt: [u8; 16] = field(&plist_of(&first), "salt").try_into().expect("salt");
            let server_public = field(&plist_of(&first), "pk");
            let wrong =
                test_client::exchange(username, b"0001", &salt, &[0x07; 32], &server_public);
            // The PIN from `pin_start` is what the server used, so the client
            // has to guess it; it cannot, and is refused.
            assert!(
                matches!(
                    session.setup_pin(&request(vec![
                        ("pk", plist::Value::Data(wrong.public_key)),
                        ("proof", plist::Value::Data(wrong.proof.to_vec())),
                    ])),
                    Err(PairingError::AuthenticationFailed)
                ),
                "attempt {attempt} must be refused"
            );
            if attempt + 1 < MAX_PIN_FAILURES {
                assert_eq!(
                    shared.pending_pin().as_deref(),
                    Some(pin.as_str()),
                    "the PIN survives {} of {MAX_PIN_FAILURES} failures",
                    attempt + 1
                );
            }
        }
        // The bound is reached: the PIN is dropped and a new one is drawn.
        assert_eq!(shared.pending_pin(), None);
        session.pin_start().expect("pin start again");
        assert!(shared.pending_pin().is_some());
    }

    #[test]
    fn pair_setup_answers_the_public_key_only_for_the_right_length() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));
        // Upstream ignores the content of the body and only checks its length.
        let reply = session.setup(&[0u8; 32]).expect("legacy setup");
        assert_eq!(body_of(&reply), shared.identity_public());
        assert_eq!(session.status, Status::Setup);

        let mut session = PairingSession::new(Arc::clone(&shared));
        assert!(matches!(
            session.setup(&[0u8; 31]),
            Ok(PairingReply::Silence)
        ));
    }

    /// Client side of `/pair-verify`: keys the test controls, so the harness
    /// never uses the function under test to build its own input.
    struct VerifyClient {
        private: [u8; 32],
        public: [u8; 32],
        identity_seed: [u8; 32],
        identity_public: [u8; 32],
    }

    impl VerifyClient {
        fn new() -> Self {
            let private = [0x21u8; 32];
            let identity_seed = [0x33u8; 32];
            Self {
                public: x25519_public(&private),
                private,
                identity_seed,
                identity_public: ed25519_public_key(&identity_seed),
            }
        }

        /// Step 1 body: a four-byte mode, the client key exchange key and the
        /// client's public key (`upstream/UxPlay/lib/raop_handlers.h:488-497`).
        fn handshake_body(&self) -> Vec<u8> {
            let mut body = vec![1u8, 0, 0, 0];
            body.extend_from_slice(&self.public);
            body.extend_from_slice(&self.identity_public);
            body
        }

        /// Decrypts the reply of step 1 and checks the receiver's signature.
        fn verify_server(&self, reply: &[u8], receiver_public: &[u8; 32]) -> Result<(), ()> {
            let server_ecdh: [u8; 32] = reply[..32].try_into().map_err(|_| ())?;
            let mut signature: [u8; 64] = reply[32..].try_into().map_err(|_| ())?;
            let shared = x25519_shared(&self.private, &server_ecdh);
            let keys = derive_pair_verify_keys(&shared);
            let mut cipher = Aes128CtrState::new(&keys.key, &keys.iv).map_err(|_| ())?;
            cipher.apply(&mut signature);
            let message = [server_ecdh, self.public].concat();
            ed25519_verify(receiver_public, &message, &signature)
                .then_some(())
                .ok_or(())
        }

        /// Body of step 2, signing the transcript and using the second block of
        /// the shared keystream.
        fn finish_body(&self, reply: &[u8]) -> Vec<u8> {
            let server_ecdh: [u8; 32] = reply[..32].try_into().expect("server key");
            let shared = x25519_shared(&self.private, &server_ecdh);
            let keys = derive_pair_verify_keys(&shared);
            let mut signature =
                ed25519_sign(&self.identity_seed, &[self.public, server_ecdh].concat());
            let mut cipher = Aes128CtrState::new(&keys.key, &keys.iv).expect("key and iv");
            let mut discarded = [0u8; 64];
            cipher.apply(&mut discarded);
            cipher.apply(&mut signature);
            let mut body = vec![0u8, 0, 0, 0];
            body.extend_from_slice(&signature);
            body
        }
    }

    #[test]
    fn pair_verify_handshakes_and_finishes() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));
        // Legacy setup puts the connection into the state pair-verify needs,
        // which is also how upstream lets a client verify the receiver.
        session.setup(&[0u8; 32]).expect("setup");

        let client = VerifyClient::new();
        let reply = session.verify(&client.handshake_body()).expect("handshake");
        let body = body_of(&reply).to_vec();
        assert_eq!(body.len(), 96);
        client
            .verify_server(&body, &shared.identity_public())
            .expect("the receiver's signature must verify");
        assert_eq!(session.status, Status::Handshake);

        let finish = client.finish_body(&body);
        assert!(matches!(
            session.verify(&finish),
            Ok(PairingReply::Octets(ref body)) if body.is_empty()
        ));
        assert_eq!(session.status, Status::Finished);
    }

    #[test]
    fn pair_verify_refuses_a_bad_signature_and_keeps_its_shape() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));
        session.setup(&[0u8; 32]).expect("setup");
        let client = VerifyClient::new();
        let reply = session.verify(&client.handshake_body()).expect("handshake");
        let body = body_of(&reply).to_vec();

        let mut finish = client.finish_body(&body);
        finish[10] ^= 0x01;
        assert!(matches!(
            session.verify(&finish),
            Ok(PairingReply::Disconnect)
        ));

        // Wrong lengths and unknown steps answer with the default reply.
        let mut session = PairingSession::new(Arc::clone(&shared));
        session.setup(&[0u8; 32]).expect("setup");
        assert!(matches!(
            session.verify(&[1, 0, 0]),
            Ok(PairingReply::Silence)
        ));
        // A body of the wrong length and an unknown mode both fall through to
        // the default reply, which upstream's `switch` has no default for.
        assert!(matches!(
            session.verify(&[1u8; 67]),
            Ok(PairingReply::Silence)
        ));
        assert!(matches!(
            session.verify(&[2u8; 68]),
            Ok(PairingReply::Silence)
        ));
        assert!(matches!(
            session.verify(&[3u8; 68]),
            Ok(PairingReply::Silence)
        ));
    }

    #[test]
    fn pair_verify_without_setup_follows_the_pin_policy() {
        // No PIN: upstream lets the client fall back to the legacy setup.
        let mut session = PairingSession::new(state(PinMode::Disabled));
        assert!(matches!(
            session.verify(&[1u8; 68]),
            Ok(PairingReply::Silence)
        ));

        // With a PIN the connection is moved into the setup state instead, and
        // the first step completes the handshake.
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));
        let client = VerifyClient::new();
        let reply = session.verify(&client.handshake_body()).expect("handshake");
        assert_eq!(body_of(&reply).len(), 96);
        assert_eq!(session.status, Status::Handshake);
    }

    #[test]
    fn pair_verify_honours_the_registration_predicate() {
        let shared = Arc::new(
            PairingState::with_random(
                IDENTITY_SEED,
                PinMode::Fixed(1234),
                Box::new(FixedRandom::new()),
            )
            .with_client_registered(|_| false),
        );
        let mut session = PairingSession::new(Arc::clone(&shared));
        let client = VerifyClient::new();
        // Upstream stops checking the list once the connection reaches its setup
        // state, so a refused client would only have to ask again; the refusal
        // has to survive the retry.
        for attempt in 0..2 {
            assert!(
                matches!(
                    session.verify(&client.handshake_body()),
                    Ok(PairingReply::Silence)
                ),
                "attempt {attempt} must be refused"
            );
        }
        assert_eq!(session.status, Status::Setup);

        // Proving the PIN is what lifts the refusal, so a client that pairs on
        // this connection can still verify on it afterwards.
        session.pin_start().expect("pin start");
        let username = b"AA:BB:CC:DD:EE:FF";
        let (exchange, _) = setup_pin_two_steps(&mut session, b"1234", username);
        let (epk, auth_tag) = seal_step_three(&exchange.session_key, &client.identity_public);
        session
            .setup_pin(&step_three_request(epk, auth_tag))
            .expect("step 3");
        let reply = session.verify(&client.handshake_body()).expect("handshake");
        assert_eq!(body_of(&reply).len(), 96);
    }

    #[test]
    fn pair_verify_accepts_a_registered_client() {
        let shared = Arc::new(
            PairingState::with_random(
                IDENTITY_SEED,
                PinMode::Fixed(1234),
                Box::new(FixedRandom::new()),
            )
            .with_client_registered(|_| true),
        );
        let mut session = PairingSession::new(Arc::clone(&shared));
        let client = VerifyClient::new();
        let reply = session.verify(&client.handshake_body()).expect("handshake");
        assert_eq!(body_of(&reply).len(), 96);
    }

    #[test]
    fn setup_pin_bounds_the_work_one_connection_can_cause() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));
        let username = b"AA:BB:CC:DD:EE:FF";
        for attempt in 0..MAX_SETUP_PIN_ATTEMPTS {
            assert!(
                session.setup_pin(&step_one_request(username)).is_ok(),
                "attempt {attempt} opens an SRP context"
            );
        }
        // Beyond the bound the receiver refuses instead of spending another
        // modular exponentiation for an unauthenticated peer.
        assert!(matches!(
            session.setup_pin(&step_one_request(username)),
            Err(PairingError::AuthenticationFailed)
        ));
    }

    #[test]
    fn session_keys_are_handed_out_only_once_and_only_after_verification() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));
        let client = VerifyClient::new();

        // Nothing on this connection, nothing to hand out.
        assert_eq!(session.authentication(), SessionAuth::None);
        assert!(session.take_session_keys().is_none());

        // The legacy setup opens the verify path but authenticates nobody, so it
        // still must not produce stream material.
        session.setup(&[0u8; 32]).expect("setup");
        assert_eq!(session.authentication(), SessionAuth::LegacySetup);
        assert!(session.take_session_keys().is_none());

        // A started key exchange is not a finished one.
        let reply = session.verify(&client.handshake_body()).expect("handshake");
        let body = body_of(&reply).to_vec();
        assert!(session.take_session_keys().is_none());

        let finish = client.finish_body(&body);
        assert!(matches!(
            session.verify(&finish),
            Ok(PairingReply::Octets(_))
        ));
        assert_eq!(session.authentication(), SessionAuth::Verified);
        let keys = session.take_session_keys().expect("verified session");
        assert_eq!(
            keys.ecdh_secret,
            airplay_core_shared_secret(&client, &body),
            "the material is the shared secret of this connection's exchange"
        );
        // Handed out once: a second media session cannot reuse it.
        assert!(session.take_session_keys().is_none());
    }

    /// Shared secret of the test client's exchange, computed from the client's
    /// private key and the receiver's public key.
    fn airplay_core_shared_secret(client: &VerifyClient, reply: &[u8]) -> [u8; X25519_KEY_SIZE] {
        let server_ecdh: [u8; X25519_KEY_SIZE] = reply[..32].try_into().expect("server key");
        x25519_shared(&client.private, &server_ecdh)
    }

    #[test]
    fn session_keys_are_not_handed_out_to_a_connection_the_predicate_refused() {
        let shared = Arc::new(
            PairingState::with_random(
                IDENTITY_SEED,
                PinMode::Fixed(1234),
                Box::new(FixedRandom::new()),
            )
            .with_client_registered(|_| false),
        );
        let mut session = PairingSession::new(Arc::clone(&shared));
        let client = VerifyClient::new();
        assert!(matches!(
            session.verify(&client.handshake_body()),
            Ok(PairingReply::Silence)
        ));
        assert_eq!(session.authentication(), SessionAuth::LegacySetup);
        assert!(session.take_session_keys().is_none());
    }

    #[test]
    fn pin_verification_is_visible_to_a_caller() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));
        session.pin_start().expect("pin start");
        let username = b"AA:BB:CC:DD:EE:FF";
        let (exchange, _) = setup_pin_two_steps(&mut session, b"1234", username);
        let (epk, auth_tag) = seal_step_three(&exchange.session_key, &[0x33; 32]);
        session
            .setup_pin(&step_three_request(epk, auth_tag))
            .expect("step 3");
        assert_eq!(session.authentication(), SessionAuth::PinVerified);
        // Proving the PIN is not the same as verifying the exchange.
        assert!(session.take_session_keys().is_none());
    }

    #[test]
    fn pair_verify_starts_one_key_exchange_per_connection() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));
        session.setup(&[0u8; 32]).expect("setup");
        let client = VerifyClient::new();
        let reply = session.verify(&client.handshake_body()).expect("handshake");
        assert_eq!(body_of(&reply).len(), 96);

        // A second key exchange would replace the keys this connection already
        // agreed on, so it is refused.
        assert!(matches!(
            session.verify(&client.handshake_body()),
            Ok(PairingReply::Silence)
        ));
        assert_eq!(session.status, Status::Handshake);
    }

    #[test]
    fn pair_verify_refuses_a_low_order_key_exchange_key() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));
        session.setup(&[0u8; 32]).expect("setup");
        let mut body = vec![1u8, 0, 0, 0];
        body.extend_from_slice(&[0u8; 32]);
        body.extend_from_slice(&ed25519_public_key(&[0x33; 32]));
        assert!(matches!(
            session.verify(&body),
            Err(PairingError::AuthenticationFailed)
        ));
    }

    #[test]
    fn debug_output_carries_no_key_material() {
        let shared = state(PinMode::Fixed(1234));
        let mut session = PairingSession::new(Arc::clone(&shared));
        session.pin_start().expect("pin start");
        let session_text = format!("{session:?}");
        let state_text = format!("{shared:?}");
        assert!(state_text.contains("identity_public"));
        assert!(!state_text.contains(&hex(&IDENTITY_SEED)));
        assert!(!session_text.contains(&hex(&IDENTITY_SEED)));
        assert!(session_text.contains("Handshake") || session_text.contains("Initial"));
        let error = PairingError::AuthenticationFailed.to_string();
        assert!(!error.contains(&hex(&IDENTITY_SEED)));
    }
}
