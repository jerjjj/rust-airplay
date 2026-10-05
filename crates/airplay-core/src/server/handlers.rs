// SPDX-License-Identifier: GPL-3.0-or-later
//! Route handlers for the network control service.
//!
//! Each handler is a port of one function in
//! `upstream/UxPlay/lib/raop_handlers.h`. A handler contributes only the status
//! code, reason phrase, body and headers its upstream counterpart adds: the
//! routing decision, the `Audio-Jack-Status`/`Server`/`CSeq` envelope and the
//! status of a route this port has not ported yet stay in [`crate::control`].

use std::sync::Arc;

use crate::control::{RequestTrace, Route, RouteHandler, RouteReply, RouteRequest};
use crate::crypto::sha512_concat;
use crate::fairplay::{FairPlaySession, HANDSHAKE_REQUEST_LEN, SETUP_REQUEST_LEN};
use crate::pairing::parse_digest_authorization;
use crate::plist::{self, Plist};

use super::pairing::{PairingError, PairingReply, PairingSession, PairingState, SessionAuth};

/// Lower-case hexadecimal, for the one challenge value this module formats.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Streams one `SETUP` request may ask for.
///
/// A real client asks for one audio stream and, while mirroring, one more; the
/// bound keeps a request from making this receiver build an unbounded answer.
const MAX_SETUP_STREAMS: usize = 4;

/// Ports a `SETUP` response may advertise.
///
/// Upstream answers the ports it has bound by the time the request arrives
/// (`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:908`, `:935-936`, `:958`,
/// `:985`). This port has no media or timing sockets yet, and a field it cannot
/// honour is left out instead of being filled with a placeholder, so a client
/// never believes it may send media somewhere nothing listens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SetupPorts {
    /// Port of the timing service, sent as `timingPort`.
    pub timing_port: Option<u16>,
    /// Port of the event channel, sent as `eventPort` (upstream sends `0`,
    /// because neither audio nor mirror mode uses it).
    pub event_port: Option<u16>,
    /// Audio data port, sent in the `type: 96` stream entry.
    pub data_port: Option<u16>,
    /// Audio control port, sent in the `type: 96` stream entry.
    pub control_port: Option<u16>,
    /// Mirror data port, sent in the `type: 110` stream entry.
    pub mirror_data_port: Option<u16>,
}

/// One stream a client asked for in `SETUP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupStream {
    /// Stream type: `96` is audio, `110` is mirroring
    /// (`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:954-983`).
    pub kind: u16,
    /// `streamConnectionID`, which the mirror stream uses to derive its own
    /// AES-CTR key (`raop_handlers.h:959-963`).
    pub stream_connection_id: Option<u64>,
}

/// What one accepted `SETUP` request hands to the session layer.
///
/// The audio key is the FairPlay-decrypted `ekey`, hashed with the connection's
/// verified pairing secret when there is one
/// (`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:848-854`). Nothing is derived
/// from an unverified exchange: the session material of a connection is produced
/// only after it finished `/pair-verify`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupRequest {
    /// Audio key, present when the request carried `ekey` and `eiv`.
    pub audio_key: Option<[u8; 16]>,
    /// Audio IV, present together with [`SetupRequest::audio_key`].
    pub audio_iv: Option<[u8; 16]>,
    /// Streams the client asked for, in request order.
    pub streams: Vec<SetupStream>,
    /// Client key of the verified pairing exchange, when this connection had one.
    pub client_identity: Option<[u8; 32]>,
    /// How far the pairing exchange of this connection got.
    ///
    /// A session layer that needs an access decision reads this instead of
    /// guessing from the presence of a key: `Verified` proves the peer holds its
    /// private key, nothing more.
    pub authentication: SessionAuth,
}

/// What a client asked to change with `SET_PARAMETER`.
///
/// Upstream hands both values straight to its media layer
/// (`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:1146-1158`); a receiver whose
/// media layer is a later stage reports them instead of pretending to act.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SetParameter {
    /// `volume: <value>`: the volume the client asked for.
    Volume(f32),
    /// `progress: <start>/<current>/<end>`, in seconds.
    Progress {
        /// Position the track started at.
        start: u32,
        /// Position the client is at.
        current: u32,
        /// Position the track ends at.
        end: u32,
    },
}

/// Callback that is told about every request a connection served.
///
/// It borrows the trace rather than owning it, so a receiver that logs requests
/// does not copy request data around.
pub type TraceCallback = Box<dyn FnMut(&RequestTrace<'_>) + Send>;

/// Model upstream advertises when the user does not override it
/// (`upstream/UxPlay/lib/global.h:21`).
pub const DEFAULT_MODEL: &str = "AppleTV3,2";

/// Version upstream reports as `sourceVersion` and in its `Server` header
/// (`upstream/UxPlay/lib/global.h:22`).
pub const DEFAULT_SOURCE_VERSION: &str = "220.68";

/// AirPlay feature bitmap upstream advertises by default.
///
/// This is `(FEATURES_2 << 32) | FEATURES_1` with the defaults of
/// `upstream/UxPlay/lib/dnssdint.h:32-34` joined as
/// `upstream/UxPlay/lib/dnssd.c:155-158` does; bit 27 (legacy pairing) is set.
pub const DEFAULT_FEATURES: u64 = 0x5A7F_FEE6;

/// Pairing identity upstream advertises (`upstream/UxPlay/lib/dnssdint.h:49`).
pub const DEFAULT_PI: &str = "2e388006-13ba-4041-9a67-25dd4a43d536";

/// `vv` upstream advertises (`upstream/UxPlay/lib/dnssdint.h:48`).
pub const DEFAULT_VV: u32 = 2;

/// Default display width in pixels (`upstream/UxPlay/lib/raop.c:620-624`).
pub const DEFAULT_WIDTH: u64 = 1920;

/// Default display height in pixels (`upstream/UxPlay/lib/raop.c:620-624`).
pub const DEFAULT_HEIGHT: u64 = 1080;

/// Default display refresh rate in hertz
/// (`upstream/UxPlay/lib/raop.c:620-624`).
pub const DEFAULT_REFRESH_RATE: f64 = 60.0;

/// Default display frame rate upstream advertises
/// (`upstream/UxPlay/lib/raop.c:620-624`).
pub const DEFAULT_MAX_FPS: u64 = 30;

/// `statusFlags` upstream always sends
/// (`upstream/UxPlay/lib/raop_handlers.h:148`).
pub const STATUS_FLAGS: u64 = 68;

/// Hard-coded display identifier upstream always sends
/// (`upstream/UxPlay/lib/raop_handlers.h:219`).
pub const DISPLAY_UUID: &str = "e0ff8a27-6738-3d56-8a16-cc53aacee925";

/// Bitmap of the single display upstream describes
/// (`upstream/UxPlay/lib/raop_handlers.h:228`).
pub const DISPLAY_FEATURES: u64 = 14;

/// Audio formats up to 5.1 surround that upstream advertises
/// (`upstream/UxPlay/lib/raop_handlers.h:198-199`).
pub const AUDIO_OUTPUT_FORMATS: u64 = 0x03FF_FFFC;

/// The identity this receiver advertises to clients.
///
/// Upstream keeps the same facts spread over its DNS-SD record, `raop_t` and
/// the `GLOBAL_MODEL`/`GLOBAL_VERSION` macros; this type collects the ones
/// `GET /info` and the upstream defaults need so a caller can describe a
/// receiver in one value.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceIdentity {
    /// Hardware address a client sees as `deviceID` and `macAddress`. Upstream
    /// formats it with `utils_hwaddr_airplay`, i.e. lower-case hex pairs
    /// separated by colons.
    pub device_id: String,
    /// Name a client shows in its device list.
    pub name: String,
    /// `model` value, [`DEFAULT_MODEL`] unless the caller overrides it.
    pub model: String,
    /// Ed25519 public key of the receiver, sent as the `pk` data value
    /// (`upstream/UxPlay/lib/raop_handlers.h:126-131`).
    ///
    /// [`RouteHandlers::with_pairing`] replaces it with
    /// [`PairingState::identity_public`], so that a client can never cache one
    /// identity from `/info` and be authenticated with another one from the
    /// pairing exchange (see `docs/ADR-0002-pairing-identity.md`). A caller that
    /// advertises a key here without pairing has to keep that invariant itself.
    pub public_key: Vec<u8>,
    /// `features` bitmap, [`DEFAULT_FEATURES`] unless the caller overrides it.
    pub features: u64,
    /// `sourceVersion` value, [`DEFAULT_SOURCE_VERSION`] by default.
    pub source_version: String,
    /// `pi` value, [`DEFAULT_PI`] by default.
    pub pi: String,
    /// `vv` value, [`DEFAULT_VV`] by default.
    pub vv: u32,
    /// `initialVolume` value. Upstream asks its audio callback for the current
    /// client volume (`upstream/UxPlay/lib/raop_handlers.h:168`).
    pub initial_volume: f64,
    /// Display width in pixels, [`DEFAULT_WIDTH`] by default.
    pub width: u64,
    /// Display height in pixels, [`DEFAULT_HEIGHT`] by default.
    pub height: u64,
    /// Display refresh rate in hertz, [`DEFAULT_REFRESH_RATE`] by default.
    pub refresh_rate: f64,
    /// Display frame rate, [`DEFAULT_MAX_FPS`] by default.
    pub max_fps: u64,
    /// Whether the display is overscanned, `false` by default.
    pub overscanned: bool,
    /// Raw `txtAirPlay` record a client asks for with
    /// `GET /info?txtAirPlay`, or `None` while service discovery has not been
    /// ported (`upstream/UxPlay/lib/raop_handlers.h:96-100`).
    pub txt_airplay: Option<Vec<u8>>,
    /// Raw `txtRAOP` record, the RAOP counterpart of [`Self::txt_airplay`].
    pub txt_raop: Option<Vec<u8>>,
}

impl DeviceIdentity {
    /// Creates an identity for `device_id` and `name` that carries `public_key`
    /// and keeps every other upstream default.
    pub fn new(device_id: impl Into<String>, name: impl Into<String>, public_key: Vec<u8>) -> Self {
        Self {
            device_id: device_id.into(),
            name: name.into(),
            public_key,
            ..Self::default()
        }
    }
}

impl Default for DeviceIdentity {
    /// Upstream's defaults with a placeholder hardware address.
    ///
    /// The hardware address identifies the receiver, so a caller that serves
    /// real clients must set [`DeviceIdentity::device_id`] and
    /// [`DeviceIdentity::public_key`]; the placeholder is only meant to keep
    /// configuration and tests readable.
    fn default() -> Self {
        Self {
            device_id: "00:00:00:00:00:00".to_owned(),
            name: "rust-airplay".to_owned(),
            model: DEFAULT_MODEL.to_owned(),
            public_key: Vec::new(),
            features: DEFAULT_FEATURES,
            source_version: DEFAULT_SOURCE_VERSION.to_owned(),
            pi: DEFAULT_PI.to_owned(),
            vv: DEFAULT_VV,
            initial_volume: 0.0,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            refresh_rate: DEFAULT_REFRESH_RATE,
            max_fps: DEFAULT_MAX_FPS,
            overscanned: false,
            txt_airplay: None,
            txt_raop: None,
        }
    }
}

/// The handlers one control connection uses.
///
/// Upstream keeps the same state in `raop_conn_t`, so one instance serves one
/// connection: the FairPlay session established by `POST /fp-setup` and the
/// pairing exchange belong to the client that opened it. The receiver-wide half
/// of pairing lives in [`PairingState`], which a caller shares between
/// connections.
pub struct RouteHandlers {
    identity: DeviceIdentity,
    fairplay: FairPlaySession,
    feedback: Option<Box<dyn FnMut() + Send>>,
    pairing: Option<PairingSession>,
    setup_ports: SetupPorts,
    setup: Option<Box<dyn FnMut(SetupRequest) + Send>>,
    parameter: Option<Box<dyn FnMut(SetParameter) + Send>>,
    trace: Option<TraceCallback>,
    authenticated: bool,
    digest_nonce: Option<[u8; 16]>,
}

impl RouteHandlers {
    /// Creates handlers that advertise `identity` and serve the routes that need
    /// no receiver-wide state.
    ///
    /// The pairing routes answer `501 Not Implemented` until a caller adds
    /// [`PairingState`] with [`RouteHandlers::with_pairing`].
    pub fn new(identity: DeviceIdentity) -> Self {
        Self {
            identity,
            fairplay: FairPlaySession::new(),
            feedback: None,
            pairing: None,
            setup_ports: SetupPorts::default(),
            setup: None,
            parameter: None,
            trace: None,
            authenticated: false,
            digest_nonce: None,
        }
    }

    /// Creates handlers that advertise `identity` and pair clients with `state`.
    ///
    /// The public key of `state` is what `/info`, `/pair-setup` and the reply of
    /// `/pair-setup-pin` answer with; it replaces
    /// [`DeviceIdentity::public_key`] so that a client can never cache one
    /// identity and be authenticated with another
    /// (`upstream/UxPlay/lib/raop.c:659-674` reads a single `pk` for all three).
    pub fn with_pairing(identity: DeviceIdentity, state: Arc<PairingState>) -> Self {
        let mut identity = identity;
        identity.public_key = state.identity_public().to_vec();
        Self {
            pairing: Some(PairingSession::new(state)),
            ..Self::new(identity)
        }
    }

    /// Sets the ports a `SETUP` response may advertise.
    pub fn set_setup_ports(&mut self, ports: SetupPorts) {
        self.setup_ports = ports;
    }

    /// Sets the callback that is told about every request this connection served.
    ///
    /// A trace carries the method, target, `CSeq`, `User-Agent`, `Content-Type`,
    /// body length and answer status of a request, and never a body, key or
    /// signature, so a receiver can be followed against a real client without
    /// printing secrets.
    pub fn set_trace_callback(&mut self, callback: impl FnMut(&RequestTrace<'_>) + Send + 'static) {
        self.trace = Some(Box::new(callback));
    }

    /// Sets the callback that receives what a `SET_PARAMETER` request asked for.
    ///
    /// Upstream applies the same values to its media session
    /// (`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:1149-1158`).
    pub fn set_parameter_callback(&mut self, callback: impl FnMut(SetParameter) + Send + 'static) {
        self.parameter = Some(Box::new(callback));
    }

    /// Sets the callback that receives what a `SETUP` request established.
    ///
    /// Upstream passes the same values straight into its media sessions
    /// (`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:926-929`); a receiver
    /// without those sessions uses the callback to keep the key for whatever
    /// plays the stream.
    pub fn set_setup_callback(&mut self, callback: impl FnMut(SetupRequest) + Send + 'static) {
        self.setup = Some(Box::new(callback));
    }

    /// The identity these handlers advertise.
    pub fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    /// Sets the callback reported for `POST /feedback`.
    ///
    /// Upstream's handler only calls `conn_feedback`
    /// (`upstream/UxPlay/lib/raop_handlers.h:1204-1213`), which is how a
    /// receiver notices that a client is still there; the response is the
    /// default `200 OK`.
    pub fn set_feedback_callback(&mut self, callback: impl FnMut() + Send + 'static) {
        self.feedback = Some(Box::new(callback));
    }
}

impl RouteHandler for RouteHandlers {
    fn handle(&mut self, route: Route, request: &RouteRequest<'_>) -> Option<RouteReply> {
        match route {
            Route::Feedback => Some(self.feedback_reply()),
            Route::Info => Some(self.info(request)),
            Route::FairPlaySetup => Some(self.fairplay_setup(request)),
            Route::PairPinStart => Some(self.pairing_reply(|session| session.pin_start())),
            Route::PairSetupPin => Some(self.pair_setup_pin(request)),
            Route::PairSetup => {
                let body = request.body;
                Some(self.pairing_reply(|session| session.setup(body)))
            }
            Route::PairVerify => {
                let body = request.body;
                Some(self.pairing_reply(|session| session.verify(body)))
            }
            Route::Setup => Some(self.setup(request)),
            Route::SetParameter => Some(self.set_parameter(request)),
            Route::GetParameter => Some(self.get_parameter(request)),
            _ => None,
        }
    }

    fn trace(&mut self, trace: &RequestTrace<'_>) {
        self.report_trace(trace);
    }
}

impl RouteHandlers {
    /// Runs one pairing step on this connection's session.
    ///
    /// Without a [`PairingState`] the route stays unimplemented, which is what
    /// the control layer reports for a route upstream serves and this port does
    /// not.
    fn pairing_reply(
        &mut self,
        step: impl FnOnce(&mut PairingSession) -> Result<PairingReply, PairingError>,
    ) -> RouteReply {
        let Some(session) = self.pairing.as_mut() else {
            return RouteReply::not_implemented();
        };
        match step(session) {
            Ok(PairingReply::Silence) => RouteReply::ok(),
            Ok(PairingReply::Plist(value)) => binary_plist(value),
            Ok(PairingReply::Octets(body)) => octet_stream(body),
            // `raop_handlers.h:533-537`: the default reply without a content
            // type, and the connection is closed afterwards.
            Ok(PairingReply::Disconnect) => RouteReply::ok().closing(),
            Err(error) => RouteReply::new(error.status(), error.reason()),
        }
    }

    /// Port of `raop_handler_pairsetup_pin`
    /// (`upstream/UxPlay/lib/raop_handlers.h:276-433`).
    fn pair_setup_pin(&mut self, request: &RouteRequest<'_>) -> RouteReply {
        // `raop_handlers.h:287-298`: upstream requires the header block to
        // mention the binary plist format and answers 470 otherwise. It searches
        // the whole head, the port only its `Content-Type`.
        let is_plist = request
            .head
            .header("content-type")
            .is_some_and(|value| value.contains("apple-binary-plist"));
        if !is_plist {
            let error = PairingError::NotPlist;
            return RouteReply::new(error.status(), error.reason());
        }
        let body = request.body;
        self.pairing_reply(|session| session.setup_pin(body))
    }
}

impl RouteHandlers {
    /// Port of `raop_handler_setup`
    /// (`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:600-1067`).
    ///
    /// This step covers what an AirPlay 1 client sends: the encrypted audio key
    /// with its IV, and the streams it wants. Later steps add the digest
    /// password branch (`:650-748`) and the media and timing sessions the
    /// handler starts (`:910-929`); a receiver without them answers the ports it
    /// actually has.
    fn setup(&mut self, request: &RouteRequest<'_>) -> RouteReply {
        // The password gate comes first, before any of the body is looked at:
        // upstream parses the property list before it challenges
        // (`raop_handlers.h:601-648`), and a receiver should not spend work on a
        // request that has not authenticated yet.
        if let Some(challenge) = self.digest_challenge(request) {
            return challenge;
        }
        // Upstream reads the body with `plist_from_bin` and carries on with an
        // empty request when that fails (`:601-644`); a malformed body is
        // refused here instead, because the alternative is a `200 OK` that
        // describes a session nobody can use.
        if !request.body.starts_with(b"bplist00") {
            return RouteReply::new(400, "Bad Request");
        }
        let Ok(body) = Plist::parse(request.body) else {
            return RouteReply::new(400, "Bad Request");
        };

        // `raop_handlers.h:785-798`: both keys are read as fixed-size buffers
        // there, without looking at the length the client sent.
        let audio_iv = match body.get_data("eiv").ok() {
            Some(iv) => match <[u8; 16]>::try_from(iv) {
                Ok(iv) => Some(iv),
                Err(_) => return RouteReply::new(400, "Bad Request"),
            },
            None => None,
        };
        let encrypted = body.get_data("ekey").ok();
        let mut client_identity = None;
        let (audio_key, audio_iv) = match (encrypted, audio_iv) {
            // A mirror-only setup carries neither, `raop_handlers.h:633`.
            (None, None) => (None, None),
            (Some(encrypted), Some(iv)) => {
                if encrypted.len() != 72 {
                    return RouteReply::new(400, "Bad Request");
                }
                let mut key = match self.fairplay.decrypt(encrypted) {
                    Ok(key) => key,
                    // Upstream logs the failure and keeps going with whatever the
                    // buffer held (`:807-813`).
                    Err(_) => return RouteReply::new(400, "Bad Request"),
                };
                if let Some(session) = self.pairing.as_mut() {
                    if let Some(keys) = session.take_session_keys() {
                        // `raop_handlers.h:848-854`: the key is hashed with the
                        // shared secret of a pairing session, which this port
                        // only produces for a connection that verified the
                        // client (`pairing.c:108-119` is what it narrows).
                        let digest = sha512_concat(&[&key, &keys.ecdh_secret]);
                        key.copy_from_slice(&digest[..16]);
                        client_identity = keys.client_identity;
                    }
                }
                (Some(key), Some(iv))
            }
            // Half a key pair is a request that cannot be answered.
            _ => return RouteReply::new(400, "Bad Request"),
        };

        // `raop_handlers.h:942-1060`: every requested stream is answered with
        // the port the receiver listens on, in request order.
        let mut streams = Vec::new();
        if let Ok(requested) = body.get_array("streams") {
            if requested.len() > MAX_SETUP_STREAMS {
                return RouteReply::new(400, "Bad Request");
            }
            for stream in requested {
                let Some(stream) = stream.as_dictionary() else {
                    return RouteReply::new(400, "Bad Request");
                };
                let Some(kind) = stream
                    .get("type")
                    .and_then(plist::Value::as_unsigned_integer)
                    .and_then(|kind| u16::try_from(kind).ok())
                else {
                    return RouteReply::new(400, "Bad Request");
                };
                let connection_id = stream
                    .get("streamConnectionID")
                    .and_then(plist::Value::as_unsigned_integer);
                // Upstream's `switch` has no default arm: an unknown type is
                // simply not answered (`:954-1050`).
                if !matches!(kind, 96 | 110) {
                    continue;
                }
                streams.push(SetupStream {
                    kind,
                    stream_connection_id: connection_id,
                });
            }
        }

        let mut dictionary = plist::Dictionary::new();
        if let Some(port) = self.setup_ports.timing_port {
            dictionary.insert("timingPort".to_owned(), u64::from(port).into());
        }
        if let Some(port) = self.setup_ports.event_port {
            dictionary.insert("eventPort".to_owned(), u64::from(port).into());
        }
        if !streams.is_empty() {
            let answer = streams
                .iter()
                .map(|stream| {
                    let mut entry = plist::Dictionary::new();
                    let port = match stream.kind {
                        110 => self.setup_ports.mirror_data_port,
                        _ => self.setup_ports.data_port,
                    };
                    if let Some(port) = port {
                        entry.insert("dataPort".to_owned(), u64::from(port).into());
                    }
                    if stream.kind == 96 {
                        if let Some(port) = self.setup_ports.control_port {
                            entry.insert("controlPort".to_owned(), u64::from(port).into());
                        }
                    }
                    entry.insert("type".to_owned(), u64::from(stream.kind).into());
                    plist::Value::Dictionary(entry)
                })
                .collect();
            dictionary.insert("streams".to_owned(), plist::Value::Array(answer));
        }
        let authentication = self
            .pairing
            .as_ref()
            .map(|session| session.authentication())
            .unwrap_or(SessionAuth::None);
        self.report_setup(SetupRequest {
            audio_key,
            audio_iv,
            streams,
            client_identity,
            authentication,
        });
        binary_plist(plist::Value::Dictionary(dictionary))
    }

    /// RFC 2617 password gate of `SETUP`, or `None` when the request may pass.
    ///
    /// Upstream keeps the same three facts on its receiver: whether the
    /// connection authenticated, the challenge it handed out, and how many
    /// attempts failed (`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:649-748`).
    /// The port keeps the nonce and the flag per connection and the failure count
    /// on the shared receiver state, so guessing across connections costs
    /// attempts. The nonce is one the port drew from its own random source
    /// instead of upstream's construction, which hashes a heap address.
    fn digest_challenge(&mut self, request: &RouteRequest<'_>) -> Option<RouteReply> {
        let state = Arc::clone(self.pairing.as_ref()?.shared());
        if !state.has_password() || self.authenticated {
            return None;
        }
        let method = request.head.method.as_str();
        if let Some(header) = request.head.header("authorization") {
            if let Some(parsed) = parse_digest_authorization(header) {
                let nonce_matches = self
                    .digest_nonce
                    .is_some_and(|nonce| hex(&nonce) == parsed.nonce);
                // The challenge is single use, whether or not it matched.
                self.digest_nonce = None;
                if nonce_matches && state.verify_password_digest(method, &parsed).is_ok() {
                    self.authenticated = true;
                    return None;
                }
            }
        }
        let mut nonce = [0u8; 16];
        if state.fill_random(&mut nonce).is_err() {
            // Without randomness there is no challenge worth sending.
            return Some(RouteReply::new(401, "Unauthorized"));
        }
        self.digest_nonce = Some(nonce);
        Some(RouteReply::new(401, "Unauthorized").with_header(
            "WWW-Authenticate",
            &format!("Digest realm=\"raop\", nonce=\"{}\"", hex(&nonce)),
        ))
    }

    /// Reports an accepted `SETUP` to the callback of this connection.
    fn report_setup(&mut self, request: SetupRequest) {
        if let Some(callback) = self.setup.as_mut() {
            callback(request);
        }
    }

    /// Port of `raop_handler_set_parameter`
    /// (`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:1125-1179`).
    ///
    /// Upstream reads the body of a `text/parameters` request with `sscanf` and
    /// hands the result to its media layer; cover art and DMAP metadata go the
    /// same way. This port parses the two values strictly, reports them, and
    /// answers `200` for the content types whose payload belongs to the media
    /// layer that does not exist yet — it does not claim to have stored them.
    fn set_parameter(&mut self, request: &RouteRequest<'_>) -> RouteReply {
        let Some(content_type) = request.head.header("content-type") else {
            // Upstream answers the same status for a request that says nothing
            // about what it carries (`:1135-1139`).
            return RouteReply::new(451, "Parameter not understood");
        };
        if content_type.trim() != "text/parameters" {
            return RouteReply::new(200, "OK");
        }
        // Upstream only looks at the start of the body (`:1146`, `:1155`), so a
        // request that carries both values changes the volume and nothing else.
        let first_line = request
            .body
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default();
        let first_line = first_line.strip_suffix(b"\r").unwrap_or(first_line);
        let Ok(line) = std::str::from_utf8(first_line) else {
            return RouteReply::new(200, "OK");
        };
        let parameter = line
            .strip_prefix("volume: ")
            .and_then(|value| value.trim().parse::<f32>().ok())
            // A value no media layer can use is ignored rather than passed on:
            // upstream's `sscanf` would leave its variable at zero.
            .filter(|volume| volume.is_finite())
            .map(SetParameter::Volume)
            .or_else(|| {
                line.strip_prefix("progress: ").and_then(|value| {
                    let mut parts = value.trim().split('/');
                    let start = parts.next()?.parse().ok()?;
                    let current = parts.next()?.parse().ok()?;
                    let end = parts.next()?.parse().ok()?;
                    parts.next().is_none().then_some(SetParameter::Progress {
                        start,
                        current,
                        end,
                    })
                })
            });
        if let Some(parameter) = parameter {
            if let Some(callback) = self.parameter.as_mut() {
                callback(parameter);
            }
        }
        RouteReply::new(200, "OK")
    }

    /// Port of `raop_handler_get_parameter`
    /// (`upstream/UxPlay@3dbf7cee:lib/raop_handlers.h:1069-1123`).
    ///
    /// A client that asks about `volume` gets the value its own player reported,
    /// which upstream reads from its media callbacks and defaults to `0.0`
    /// without them (`:1093-1097`). This port has no media layer, so it answers
    /// that default instead of inventing a level.
    fn get_parameter(&mut self, request: &RouteRequest<'_>) -> RouteReply {
        let Some(content_type) = request.head.header("content-type") else {
            return RouteReply::new(451, "Parameter not understood");
        };
        if content_type.trim() != "text/parameters" {
            return RouteReply::new(200, "OK");
        }
        if request.body.starts_with(b"volume") {
            return RouteReply::new(200, "OK")
                .with_header("Content-Type", "text/parameters")
                .with_body(format!("volume: {:9.6}\r\n", 0.0_f32).into_bytes());
        }
        RouteReply::new(200, "OK")
    }

    /// Reports one served request to the trace callback of this connection.
    fn report_trace(&mut self, trace: &RequestTrace<'_>) {
        if let Some(callback) = self.trace.as_mut() {
            callback(trace);
        }
    }

    /// Port of `raop_handler_feedback`
    /// (`upstream/UxPlay/lib/raop_handlers.h:1204-1213`).
    fn feedback_reply(&mut self) -> RouteReply {
        if let Some(callback) = self.feedback.as_mut() {
            callback();
        }
        RouteReply::ok()
    }

    /// Port of `raop_handler_info`
    /// (`upstream/UxPlay/lib/raop_handlers.h:36-249`).
    ///
    /// Three request shapes reach this handler: a plist `qualifier` request that
    /// asks for a DNS-SD TXT record, a full `GET /info`, and a Bluetooth LE
    /// discovery request without a `CSeq`.
    fn info(&self, request: &RouteRequest<'_>) -> RouteReply {
        let mut dictionary = plist::Dictionary::new();
        let has_cseq = request.head.header("cseq").is_some();
        let content_type = request.head.header("content-type");

        // `raop_handlers.h:64-94`: the client names the TXT record it wants,
        // either in a binary plist qualifier or, for Bluetooth LE discovery
        // without a CSeq, in the URL.
        let mut add_airplay_txt = false;
        let mut add_raop_txt = false;
        if let Some(content_type) = content_type {
            if content_type.contains("application/x-apple-binary-plist") {
                match qualifier(request.body).as_deref() {
                    Some("txtAirPlay") => add_airplay_txt = true,
                    Some("txtRAOP") => add_raop_txt = true,
                    _ => {}
                }
            }
        } else if !has_cseq {
            add_airplay_txt = request.head.target.contains("txtAirPlay");
            add_raop_txt = request.head.target.contains("txtRAOP");
        }
        if add_airplay_txt {
            let txt = self.identity.txt_airplay.clone().unwrap_or_default();
            dictionary.insert("txtAirPlay".to_owned(), plist::Value::Data(txt));
        }
        if add_raop_txt {
            let txt = self.identity.txt_raop.clone().unwrap_or_default();
            dictionary.insert("txtRAOP".to_owned(), plist::Value::Data(txt));
        }
        // `raop_handlers.h:108-111`: a request that asked for a TXT record gets
        // nothing else, because it is the Bluetooth LE discovery path.
        if content_type.is_some() {
            return binary_plist(plist::Value::Dictionary(dictionary));
        }

        dictionary.insert(
            "deviceID".to_owned(),
            self.identity.device_id.clone().into(),
        );
        dictionary.insert(
            "macAddress".to_owned(),
            self.identity.device_id.clone().into(),
        );
        dictionary.insert(
            "pk".to_owned(),
            plist::Value::Data(self.identity.public_key.clone()),
        );
        dictionary.insert("features".to_owned(), self.identity.features.into());
        dictionary.insert("name".to_owned(), self.identity.name.clone().into());
        dictionary.insert("pi".to_owned(), self.identity.pi.clone().into());
        dictionary.insert("vv".to_owned(), u64::from(self.identity.vv).into());
        dictionary.insert("statusFlags".to_owned(), STATUS_FLAGS.into());
        dictionary.insert("keepAliveLowPower".to_owned(), 1u64.into());
        dictionary.insert(
            "sourceVersion".to_owned(),
            self.identity.source_version.clone().into(),
        );
        dictionary.insert("keepAliveSendStatsAsBody".to_owned(), true.into());
        dictionary.insert("model".to_owned(), self.identity.model.clone().into());

        // `raop_handlers.h:163-166`: the Bluetooth LE response stops here.
        if !has_cseq {
            return binary_plist(plist::Value::Dictionary(dictionary));
        }

        dictionary.insert(
            "initialVolume".to_owned(),
            self.identity.initial_volume.into(),
        );
        dictionary.insert("audioLatencies".to_owned(), audio_latencies());
        dictionary.insert("audioFormats".to_owned(), audio_formats());
        dictionary.insert("displays".to_owned(), displays(&self.identity));
        binary_plist(plist::Value::Dictionary(dictionary))
    }

    /// Port of `raop_handler_fpsetup`
    /// (`upstream/UxPlay/lib/raop_handlers.h:544-589`).
    ///
    /// A 16-byte challenge starts a session and is answered with a 142-byte
    /// reply, a 164-byte challenge completes it and is answered with 32 bytes.
    /// Any other length is a client error upstream only logs
    /// (`raop_handlers.h:585-587`), and a version or mode this port does not
    /// implement answers `501 Not Implemented` like upstream's version check
    /// (`raop_handlers.h:558`).
    fn fairplay_setup(&mut self, request: &RouteRequest<'_>) -> RouteReply {
        let body = request.body;
        if body.len() == SETUP_REQUEST_LEN {
            match self.fairplay.setup(body) {
                Ok(response) => octet_stream(response.to_vec()),
                Err(_) => RouteReply::not_implemented(),
            }
        } else if body.len() == HANDSHAKE_REQUEST_LEN {
            match self.fairplay.handshake(body) {
                Ok(response) => octet_stream(response.to_vec()),
                Err(_) => RouteReply::not_implemented(),
            }
        } else {
            RouteReply::ok()
        }
    }
}

/// `qualifier` array of a `GET /info` request, first string entry
/// (`upstream/UxPlay/lib/raop_handlers.h:73-77`).
fn qualifier(body: &[u8]) -> Option<String> {
    let request = Plist::parse(body).ok()?;
    let first = request.get_array("qualifier").ok()?.first()?;
    first.as_string().map(str::to_owned)
}

/// Wraps `value` into a binary plist response
/// (`upstream/UxPlay/lib/raop_handlers.h:246-248`).
///
/// Encoding a dictionary of strings, integers, reals, booleans and data cannot
/// fail, so the `500` is a safety net rather than an expected answer: a client
/// must never receive a truncated body it would parse as a valid plist.
fn binary_plist(value: plist::Value) -> RouteReply {
    match Plist::from_value(value).encode_binary() {
        Ok(body) => RouteReply::ok()
            .with_header("Content-Type", "application/x-apple-binary-plist")
            .with_body(body),
        Err(_) => RouteReply::new(500, "Internal Server Error"),
    }
}

/// FairPlay reply with an opaque body
/// (`upstream/UxPlay/lib/raop_handlers.h:549-570`).
fn octet_stream(body: Vec<u8>) -> RouteReply {
    RouteReply::ok()
        .with_header("Content-Type", "application/octet-stream")
        .with_body(body)
}

/// `audioLatencies` array (`upstream/UxPlay/lib/raop_handlers.h:171-193`).
fn audio_latencies() -> plist::Value {
    let default = |audio_type_identifier: u64| {
        plist::dictionary([
            ("type", plist::Value::from(audio_type_identifier)),
            ("inputLatencyMicros", plist::Value::from(0u64)),
            ("audioType", plist::Value::from("default")),
            ("outputLatencyMicros", plist::Value::from(false)),
        ])
    };
    plist::Value::Array(vec![default(100), default(101)])
}

/// `audioFormats` array (`upstream/UxPlay/lib/raop_handlers.h:195-213`).
fn audio_formats() -> plist::Value {
    let format = |audio_type_identifier: u64| {
        plist::dictionary([
            (
                "audioOutputFormats",
                plist::Value::from(AUDIO_OUTPUT_FORMATS),
            ),
            ("type", plist::Value::from(audio_type_identifier)),
            (
                "audioInputFormats",
                plist::Value::from(AUDIO_OUTPUT_FORMATS),
            ),
        ])
    };
    plist::Value::Array(vec![format(100), format(101)])
}

/// `displays` array (`upstream/UxPlay/lib/raop_handlers.h:215-243`).
///
/// The single entry describes the virtual display the receiver renders to.
fn displays(identity: &DeviceIdentity) -> plist::Value {
    // Upstream sends the frame interval and divides by the refresh rate
    // (`raop_handlers.h:225`); a rate that is not positive would divide by zero,
    // so the port reports `0.0` for an interval it cannot compute.
    let refresh_interval = if identity.refresh_rate > 0.0 {
        1.0 / identity.refresh_rate
    } else {
        0.0
    };
    let display = plist::dictionary([
        ("uuid", plist::Value::from(DISPLAY_UUID)),
        ("widthPhysical", plist::Value::from(0u64)),
        ("heightPhysical", plist::Value::from(0u64)),
        ("width", plist::Value::from(identity.width)),
        ("height", plist::Value::from(identity.height)),
        ("widthPixels", plist::Value::from(identity.width)),
        ("heightPixels", plist::Value::from(identity.height)),
        ("rotation", plist::Value::from(false)),
        ("refreshRate", plist::Value::from(refresh_interval)),
        ("maxFPS", plist::Value::from(identity.max_fps)),
        ("overscanned", plist::Value::from(identity.overscanned)),
        ("features", plist::Value::from(DISPLAY_FEATURES)),
    ]);
    plist::Value::Array(vec![display])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{MessageHead, parse_request_head};

    /// Parses `method target` plus `extra` headers and `body` the way the control
    /// layer does, and returns the head with the body slice the handler sees.
    fn parse(method_target: &str, extra: &str, body: &[u8]) -> (MessageHead, Vec<u8>) {
        let mut bytes = format!(
            "{method_target} RTSP/1.0\r\n{extra}Content-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        bytes.extend_from_slice(body);
        let head = parse_request_head(&bytes).unwrap();
        let remaining = bytes[head.consumed..].to_vec();
        (head, remaining)
    }

    fn info_reply(handlers: &RouteHandlers, head: &MessageHead, body: &[u8]) -> RouteReply {
        handlers.info(&RouteRequest { head, body })
    }

    #[test]
    fn info_answers_the_full_device_record() {
        let identity = DeviceIdentity::new("aa:bb:cc:dd:ee:ff", "Test Receiver", vec![7; 32]);
        let handlers = RouteHandlers::new(identity);
        let (head, body) = parse("GET /info", "CSeq: 1\r\n", &[]);
        let reply = info_reply(&handlers, &head, &body);

        assert_eq!(reply.status, 200);
        assert_eq!(
            reply.header("Content-Type"),
            Some("application/x-apple-binary-plist")
        );
        let parsed = Plist::parse(&reply.body).unwrap();
        assert_eq!(parsed.get_string("deviceID").unwrap(), "aa:bb:cc:dd:ee:ff");
        assert_eq!(
            parsed.get_string("macAddress").unwrap(),
            "aa:bb:cc:dd:ee:ff"
        );
        assert_eq!(parsed.get_data("pk").unwrap(), vec![7; 32]);
        assert_eq!(parsed.get_unsigned("features").unwrap(), DEFAULT_FEATURES);
        assert_eq!(parsed.get_unsigned("statusFlags").unwrap(), STATUS_FLAGS);
        assert_eq!(parsed.get_unsigned("keepAliveLowPower").unwrap(), 1);
        assert!(parsed.get_boolean("keepAliveSendStatsAsBody").unwrap());
        assert_eq!(parsed.get_string("sourceVersion").unwrap(), "220.68");
        assert_eq!(parsed.get_array("audioLatencies").unwrap().len(), 2);
        assert_eq!(parsed.get_array("audioFormats").unwrap().len(), 2);
        assert_eq!(parsed.get_array("displays").unwrap().len(), 1);
    }

    #[test]
    fn info_answers_only_the_requested_txt_record() {
        let mut identity = DeviceIdentity::new("aa:bb:cc:dd:ee:ff", "Test", Vec::new());
        identity.txt_airplay = Some(b"txt".to_vec());
        let handlers = RouteHandlers::new(identity);

        let mut request_value = plist::Dictionary::new();
        request_value.insert(
            "qualifier".to_owned(),
            plist::Value::Array(vec!["txtAirPlay".into()]),
        );
        let body = Plist::from_value(plist::Value::Dictionary(request_value))
            .encode_binary()
            .unwrap();
        let (head, body) = parse(
            "GET /info",
            "CSeq: 1\r\nContent-Type: application/x-apple-binary-plist\r\n",
            &body,
        );
        let reply = info_reply(&handlers, &head, &body);

        let parsed = Plist::parse(&reply.body).unwrap();
        assert_eq!(parsed.get_data("txtAirPlay").unwrap(), b"txt");
        assert!(!parsed.has_key("deviceID"));
    }

    #[test]
    fn bluetooth_le_info_omits_the_session_values() {
        let handlers = RouteHandlers::new(DeviceIdentity::default());
        let (head, body) = parse("GET /info?txtAirPlay", "", &[]);
        let reply = info_reply(&handlers, &head, &body);

        let parsed = Plist::parse(&reply.body).unwrap();
        assert!(parsed.has_key("txtAirPlay"));
        assert!(parsed.has_key("model"));
        assert!(!parsed.has_key("initialVolume"));
        assert!(!parsed.has_key("displays"));
    }

    #[test]
    fn fairplay_setup_rejects_unknown_versions_and_lengths() {
        let mut handlers = RouteHandlers::new(DeviceIdentity::default());

        let (head, body) = parse("POST /fp-setup", "CSeq: 1\r\n", &[0; SETUP_REQUEST_LEN]);
        let rejected = handlers.fairplay_setup(&RouteRequest {
            head: &head,
            body: &body,
        });
        // Version byte 4 is 0, which is not the FairPlay version upstream accepts.
        assert_eq!(rejected.status, 501);

        let (head, body) = parse("POST /fp-setup", "CSeq: 1\r\n", &[0; 5]);
        let ignored = handlers.fairplay_setup(&RouteRequest {
            head: &head,
            body: &body,
        });
        assert_eq!(ignored.status, 200);
        assert!(ignored.body.is_empty());
    }
}
