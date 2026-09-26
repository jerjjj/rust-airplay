// SPDX-License-Identifier: GPL-3.0-or-later
//! RTSP/HTTP control-plane routing and per-connection session state.
//!
//! [`dispatch`] mirrors the method and URL table of
//! `upstream/UxPlay/lib/raop.c:406-478`. [`ControlSession`] adds the
//! per-connection state that upstream keeps in `raop_conn_t`, and
//! [`ControlResponse`] ports the response framing of
//! `upstream/UxPlay/lib/http_response.c:71-170`. Socket I/O, service discovery
//! and the media sessions a `SETUP` request describes stay outside this module.
//!
//! # Documented differences from upstream
//!
//! * A request this module cannot parse is rejected with a status upstream also
//!   uses: `400 Bad Request` for a malformed start line, malformed headers, an
//!   unusable `Content-Length` or a body past [`MAX_CONTROL_BODY`], and
//!   `431 Request Header Fields Too Large` for a head past
//!   [`crate::wire::DEFAULT_MAX_HEAD`] (`upstream/UxPlay/lib/raop.c:232-238`).
//!   Upstream's httpd drops a request whose head it cannot parse; the port
//!   answers instead of failing silently.
//! * Every rejection is a bare status line. Upstream's `431` path returns
//!   before it adds anything (`raop.c:236-238`), and the port extends that to
//!   all rejections so an unusable request never influences a response header.
//! * Routes upstream serves but this port has not ported answer `501 Not
//!   Implemented` (see [`Route::is_ported`]).
//! * `ANNOUNCE` is an extension: upstream has no such route, and the port keeps
//!   it out of [`PUBLIC_METHODS`] so it is never advertised.
//! * [`ControlState`] is this port's summary of the RTSP lifecycle. Upstream
//!   tracks a connection type plus `raop_rtp`/`raop_rtp_mirror` handles instead,
//!   so this state is informational and never rejects a request.
//! * `SETUP`, `RECORD`, `FLUSH` and `TEARDOWN` are answered with the
//!   control-plane envelope only: the plist streams, keys and RTP ports they
//!   describe, and the `Authorization`/digest handling of a real `SETUP`
//!   (`upstream/UxPlay/lib/raop_handlers.h:624-1067`), land with the network
//!   service work (`docs/S3_WORK_PLAN.md`, W5).

use std::fmt;

use crate::wire::{
    DEFAULT_MAX_HEAD, MessageHead, ParseError, parse_request_head_with_maximum, split_body,
};

/// Largest request body the control plane accepts.
///
/// Bodies this large are rejected before they are buffered; upstream does not
/// bound them at all, so the bound is a documented port limit.
pub const MAX_CONTROL_BODY: usize = 64 * 1024;

/// Value of the `Server` header.
///
/// Upstream sends `"AirTunes/" GLOBAL_VERSION` (`upstream/UxPlay/lib/raop.c:489`)
/// and `upstream/UxPlay/lib/global.h:22` defines `GLOBAL_VERSION` as `220.68`,
/// so the port keeps that wire identity instead of inventing a version string.
pub const SERVER_HEADER_VALUE: &str = "AirTunes/220.68";

/// `Audio-Jack-Status` used by every RTSP response that carries a `CSeq`,
/// except `RECORD` (`upstream/UxPlay/lib/raop.c:338-340`).
pub const AUDIO_JACK_STATUS: &str = "connected; type=digital";

/// `Audio-Jack-Status` sent by the `RECORD` handler
/// (`upstream/UxPlay/lib/raop_handlers.h:1226`).
pub const AUDIO_JACK_STATUS_RECORD: &str = "connected; type=analog";

/// `Public` header value for `OPTIONS`
/// (`upstream/UxPlay/lib/raop_handlers.h:596`).
pub const PUBLIC_METHODS: &str =
    "SETUP, RECORD, FLUSH, TEARDOWN, OPTIONS, GET_PARAMETER, SET_PARAMETER";

/// Sample rate of every audio format upstream supports
/// (`upstream/UxPlay/lib/raop_handlers.h:27`).
pub const AUDIO_SAMPLE_RATE: u32 = 44_100;

/// Audio delay upstream starts with, in microseconds
/// (`upstream/UxPlay/lib/raop.c:639`).
pub const DEFAULT_AUDIO_DELAY_MICROS: u32 = 250_000;

/// Largest audio delay upstream accepts, in microseconds
/// (`upstream/UxPlay/lib/raop.c:759-761`).
pub const MAX_AUDIO_DELAY_MICROS: u32 = 10 * 1_000_000;

/// Converts an audio delay into the sample count of the `Audio-Latency` header.
///
/// This is `audio_delay_micros * AUDIO_SAMPLE_RATE / 1_000_000` exactly as
/// `upstream/UxPlay/lib/raop_handlers.h:1222` computes it. The multiplication
/// runs in 64 bits and the conversion falls back to `u32::MAX`, so no input can
/// wrap or panic.
pub fn audio_latency_samples(audio_delay_micros: u32) -> u32 {
    let samples = u64::from(audio_delay_micros) * u64::from(AUDIO_SAMPLE_RATE) / 1_000_000;
    u32::try_from(samples).unwrap_or(u32::MAX)
}

/// One entry of the upstream control-plane route table.
///
/// Names follow the upstream handler that serves the route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// `POST /feedback`
    Feedback,
    /// `POST /pair-pin-start`
    PairPinStart,
    /// `POST /pair-setup-pin`
    PairSetupPin,
    /// `POST /pair-setup`
    PairSetup,
    /// `POST /pair-verify`
    PairVerify,
    /// `POST /fp-setup`
    FairPlaySetup,
    /// `POST /audioMode`
    AudioMode,
    /// `GET` whose URL contains `/info` (upstream matches with `strstr`)
    Info,
    /// `OPTIONS`, any target
    Options,
    /// `SETUP`, any target
    Setup,
    /// `GET_PARAMETER`, any target
    GetParameter,
    /// `SET_PARAMETER`, any target
    SetParameter,
    /// `RECORD`, any target
    Record,
    /// `FLUSH`, any target
    Flush,
    /// `TEARDOWN`, any target
    Teardown,
    /// `POST /reverse`
    Reverse,
    /// `POST /play`
    Play,
    /// `POST /getProperty?...`
    GetProperty,
    /// `POST /scrub?...`
    Scrub,
    /// `POST /rate?...`
    Rate,
    /// `POST /stop`
    Stop,
    /// `POST /action`
    Action,
    /// `POST /fp-setup2`
    FairPlaySetup2,
    /// `GET /server-info`
    ServerInfo,
    /// `GET /playback-info`
    PlaybackInfo,
    /// `PUT /setProperty?...`
    SetProperty,
    /// HLS request: a `Host` header without `CSeq` and without
    /// `X-Apple-Session-ID` (`upstream/UxPlay/lib/raop.c:270`).
    Hls,
    /// `ANNOUNCE`, which upstream does not dispatch at all.
    ///
    /// This port keeps it as a documented extension so a session can be
    /// announced before `SETUP`; it is deliberately absent from
    /// [`PUBLIC_METHODS`] because upstream never advertises it.
    Announce,
}

impl Route {
    /// Whether this port implements the route's control-plane behavior.
    ///
    /// An unported route answers `501 Not Implemented` rather than the upstream
    /// handler's `200 OK`, so an unimplemented handler can never be mistaken for
    /// success. `SETUP`, `RECORD`, `FLUSH` and `TEARDOWN` are ported as the RTSP
    /// envelope only; their plist and media handling lands with W5.
    pub fn is_ported(self) -> bool {
        matches!(
            self,
            Self::Options
                | Self::Setup
                | Self::Record
                | Self::Flush
                | Self::Teardown
                | Self::Announce
        )
    }
}

/// Request facts the upstream dispatcher keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestFacts<'a> {
    /// Request method as sent.
    pub method: &'a str,
    /// Request target as sent.
    pub target: &'a str,
    /// Protocol as sent, for example `RTSP/1.0`.
    pub version: &'a str,
    /// Whether a `CSeq` header is present.
    pub has_cseq: bool,
    /// Whether a `Host` header is present.
    pub has_host: bool,
    /// Whether an `X-Apple-Session-ID` header is present.
    pub has_session_id: bool,
}

impl<'a> RequestFacts<'a> {
    /// Derives the routing facts from one parsed request head.
    pub fn from_head(head: &'a MessageHead) -> Self {
        Self {
            method: &head.method,
            target: &head.target,
            version: &head.version,
            has_cseq: head.header("cseq").is_some(),
            has_host: head.header("host").is_some(),
            has_session_id: head.header("x-apple-session-id").is_some(),
        }
    }

    /// Whether this is a response to a Bluetooth LE beacon advertisement.
    ///
    /// `upstream/UxPlay/lib/raop.c:257`: an `RTSP/1.0` request without `CSeq`
    /// that mentions `txtAirPlay` or `txtRAOP` is such a response, and upstream
    /// does not ignore it even when HLS support is off.
    pub fn is_ble_beacon_response(self) -> bool {
        self.version == "RTSP/1.0"
            && !self.has_cseq
            && (self.target.contains("txtAirPlay") || self.target.contains("txtRAOP"))
    }

    /// Whether this is an HLS request (`upstream/UxPlay/lib/raop.c:270`).
    pub fn is_hls_request(self) -> bool {
        self.has_host && !self.has_cseq && !self.has_session_id
    }
}

/// Result of one upstream route lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    /// Upstream serves the route and this port implements it.
    Ported(Route),
    /// Upstream serves the route but this port has not ported it, so the
    /// response is `501 Not Implemented`.
    NotImplemented(Route),
    /// Upstream selected no handler and answers its default `200 OK` with no body.
    Unhandled,
    /// Upstream answers `501 Not Implemented` for an unknown RTSP method
    /// (`upstream/UxPlay/lib/raop.c:442-444`).
    UnknownRtspMethod,
}

/// Looks up one request in the upstream route table.
///
/// Upstream compares the protocol string exactly, so `RTSP/2.0` or any other
/// version selects no handler. The HLS rule precedes both protocol tables
/// (`upstream/UxPlay/lib/raop.c:406`, `:445-478`).
pub fn dispatch(facts: &RequestFacts<'_>) -> Dispatch {
    if facts.is_hls_request() {
        return classify(Route::Hls);
    }
    match facts.version {
        "RTSP/1.0" => dispatch_rtsp(facts),
        "HTTP/1.1" => dispatch_http(facts),
        _ => Dispatch::Unhandled,
    }
}

fn classify(route: Route) -> Dispatch {
    if route.is_ported() {
        Dispatch::Ported(route)
    } else {
        Dispatch::NotImplemented(route)
    }
}

fn classify_rtsp_post(target: &str) -> Dispatch {
    let route = match target {
        "/feedback" => Route::Feedback,
        "/pair-pin-start" => Route::PairPinStart,
        "/pair-setup-pin" => Route::PairSetupPin,
        "/pair-setup" => Route::PairSetup,
        "/pair-verify" => Route::PairVerify,
        "/fp-setup" => Route::FairPlaySetup,
        "/audioMode" => Route::AudioMode,
        _ => return Dispatch::Unhandled,
    };
    classify(route)
}

fn dispatch_rtsp(facts: &RequestFacts<'_>) -> Dispatch {
    match facts.method {
        "POST" => classify_rtsp_post(facts.target),
        "GET" => {
            if facts.target.contains("/info") {
                classify(Route::Info)
            } else {
                Dispatch::Unhandled
            }
        }
        "OPTIONS" => classify(Route::Options),
        "SETUP" => classify(Route::Setup),
        "GET_PARAMETER" => classify(Route::GetParameter),
        "SET_PARAMETER" => classify(Route::SetParameter),
        "RECORD" => classify(Route::Record),
        "FLUSH" => classify(Route::Flush),
        "TEARDOWN" => classify(Route::Teardown),
        "ANNOUNCE" => classify(Route::Announce),
        _ => Dispatch::UnknownRtspMethod,
    }
}

fn dispatch_http(facts: &RequestFacts<'_>) -> Dispatch {
    let route = match facts.method {
        "POST" => {
            let target = facts.target;
            if target == "/reverse" {
                Route::Reverse
            } else if target == "/play" {
                Route::Play
            } else if target.starts_with("/getProperty?") {
                Route::GetProperty
            } else if target.starts_with("/scrub?") {
                Route::Scrub
            } else if target.starts_with("/rate?") {
                Route::Rate
            } else if target == "/stop" {
                Route::Stop
            } else if target == "/action" {
                Route::Action
            } else if target == "/fp-setup2" {
                Route::FairPlaySetup2
            } else {
                return Dispatch::Unhandled;
            }
        }
        "GET" => match facts.target {
            "/server-info" => Route::ServerInfo,
            "/playback-info" => Route::PlaybackInfo,
            _ => return Dispatch::Unhandled,
        },
        "PUT" if facts.target.starts_with("/setProperty?") => Route::SetProperty,
        // Upstream has no 501 branch for HTTP/1.1: an unknown method leaves the
        // default 200 response untouched (`upstream/UxPlay/lib/raop.c:445-475`).
        _ => return Dispatch::Unhandled,
    };
    classify(route)
}

/// Lifecycle state this port observes for one connection.
///
/// The transitions are informational: upstream's routing layer is stateless, so
/// no request is ever rejected because of this state
/// (`upstream/UxPlay/lib/raop.c:406-478`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ControlState {
    /// Nothing has been announced or set up yet.
    #[default]
    New,
    /// `ANNOUNCE` was seen. Upstream has no such route; the port keeps the
    /// announcement as an extension.
    Announced,
    /// `SETUP` was seen.
    Setup,
    /// `RECORD` was seen after `SETUP`.
    Recording,
    /// `TEARDOWN` was seen; upstream also closes the connection
    /// (`upstream/UxPlay/lib/raop_handlers.h:1284`).
    Closed,
}

/// One response to write back on the connection.
///
/// The layout follows `upstream/UxPlay/lib/http_response.c:71-170`: the status
/// line echoes the request protocol, headers keep insertion order, and
/// `Content-Length` is appended last and only when there is a body or a
/// `Content-Type` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlResponse {
    /// Protocol echoed in the status line, for example `RTSP/1.0`.
    ///
    /// `upstream/UxPlay/lib/raop.c:335` initializes the response with the
    /// request's protocol, so an HTTP/1.1 peer is answered with `HTTP/1.1`.
    pub protocol: String,
    /// Numeric status code.
    pub status: u16,
    /// Reason phrase sent after the status code.
    pub reason: &'static str,
    /// Header fields in the order they must be written.
    pub headers: Vec<(String, String)>,
    /// Response body, empty for a header-only response.
    pub body: Vec<u8>,
}

impl ControlResponse {
    /// Builds an empty response whose status line echoes `protocol`.
    pub fn new(protocol: &str, status: u16, reason: &'static str) -> Self {
        Self {
            protocol: protocol.to_owned(),
            status,
            reason,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// First value of `name`, matched case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Serializes the response exactly as `http_response_finish` does.
    ///
    /// `upstream/UxPlay/lib/http_response.c:131-170` appends `Content-Length`
    /// and the body when there is a body, appends `Content-Length: 0` when there
    /// is no body but a `Content-Type` header was set, and otherwise terminates
    /// the head with an empty line and no `Content-Length` at all.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut output = Vec::with_capacity(128 + self.body.len());
        output.extend_from_slice(self.protocol.as_bytes());
        output.extend_from_slice(b" ");
        output.extend_from_slice(self.status.to_string().as_bytes());
        output.extend_from_slice(b" ");
        output.extend_from_slice(self.reason.as_bytes());
        output.extend_from_slice(b"\r\n");
        for (name, value) in &self.headers {
            output.extend_from_slice(name.as_bytes());
            output.extend_from_slice(b": ");
            output.extend_from_slice(value.as_bytes());
            output.extend_from_slice(b"\r\n");
        }
        if self.body.is_empty() {
            if self.header("content-type").is_some() {
                output.extend_from_slice(b"Content-Length: 0\r\n");
            }
            output.extend_from_slice(b"\r\n");
        } else {
            output.extend_from_slice(b"Content-Length: ");
            output.extend_from_slice(self.body.len().to_string().as_bytes());
            output.extend_from_slice(b"\r\n\r\n");
            output.extend_from_slice(&self.body);
        }
        output
    }
}

/// A request the control plane cannot act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    /// The request head could not be parsed. Answered with `400 Bad Request`.
    Parse(ParseError),
    /// The head block exceeded [`DEFAULT_MAX_HEAD`]. Answered with
    /// `431 Request Header Fields Too Large`, the code upstream uses for
    /// overlong request headers (`upstream/UxPlay/lib/raop.c:232-238`).
    HeadTooLarge {
        /// Size of the head that was rejected.
        declared: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// The declared body exceeded [`MAX_CONTROL_BODY`]. Answered with
    /// `400 Bad Request`; upstream bounds no body, so this is a port limit.
    BodyTooLarge {
        /// Declared body size.
        declared: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// The request carried an `X-Apple-Session-ID` that differs from the one
    /// this connection already used, so it belongs to another session.
    ///
    /// `upstream/UxPlay/lib/raop.c:400-402` asserts the same equality; the port
    /// answers `400 Bad Request` instead of aborting.
    SessionMismatch,
}

impl ControlError {
    /// Status code and reason phrase upstream associates with this failure.
    pub fn status(&self) -> (u16, &'static str) {
        match self {
            Self::HeadTooLarge { .. } => (431, "Request Header Fields Too Large"),
            Self::Parse(_) | Self::BodyTooLarge { .. } | Self::SessionMismatch => {
                (400, "Bad Request")
            }
        }
    }

    /// Builds the bare rejection response for this failure.
    ///
    /// The response carries no headers: upstream's `431` path returns before any
    /// header is added (`upstream/UxPlay/lib/raop.c:236-238`), and the port
    /// never echoes fields of a request it rejected. `protocol` is the protocol
    /// to put in the status line; a caller that could not parse the head at all
    /// passes the protocol it expects.
    pub fn to_response(&self, protocol: &str) -> ControlResponse {
        let (status, reason) = self.status();
        ControlResponse::new(protocol, status, reason)
    }
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(f),
            Self::HeadTooLarge { declared, maximum } => {
                write!(
                    f,
                    "request head of {declared} bytes exceeds maximum {maximum}"
                )
            }
            Self::BodyTooLarge { declared, maximum } => {
                write!(
                    f,
                    "request body of {declared} bytes exceeds maximum {maximum}"
                )
            }
            Self::SessionMismatch => {
                f.write_str("X-Apple-Session-ID does not match the connection session")
            }
        }
    }
}

impl std::error::Error for ControlError {}

impl From<ParseError> for ControlError {
    fn from(error: ParseError) -> Self {
        Self::Parse(error)
    }
}

/// What a caller must do with one handled request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlOutcome {
    /// Write this response back on the connection.
    Respond(ControlResponse),
    /// Write nothing and keep the connection open.
    ///
    /// Upstream drops the request in two cases: an unusable `CSeq` value
    /// (`upstream/UxPlay/lib/raop.c:247-250`) and a request without `CSeq` while
    /// HLS support is off and the request is not a BLE beacon response
    /// (`upstream/UxPlay/lib/raop.c:263-266`). Its httpd then sends no response
    /// at all (`upstream/UxPlay/lib/httpd.c:587-612`).
    Ignored,
}

/// Per-connection control-plane state.
#[derive(Debug)]
pub struct ControlSession {
    state: ControlState,
    session_id: Option<String>,
    hls_support: bool,
    audio_delay_micros: u32,
}

impl Default for ControlSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ControlSession {
    /// Creates a session with upstream's defaults: HLS support off and a
    /// [`DEFAULT_AUDIO_DELAY_MICROS`] audio delay.
    pub fn new() -> Self {
        Self {
            state: ControlState::New,
            session_id: None,
            hls_support: false,
            audio_delay_micros: DEFAULT_AUDIO_DELAY_MICROS,
        }
    }

    /// Current lifecycle state.
    pub fn state(&self) -> ControlState {
        self.state
    }

    /// `X-Apple-Session-ID` this connection is bound to, once one was seen.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Enables HLS request handling, mirroring upstream's `-hls` option.
    ///
    /// The port implements no HLS handler, so enabling this only turns the
    /// silent drop of a CSeq-less request with a `Host` header into an explicit
    /// `501 Not Implemented` for [`Route::Hls`].
    pub fn set_hls_support(&mut self, enabled: bool) {
        self.hls_support = enabled;
    }

    /// Sets the audio delay reported by `RECORD` in `Audio-Latency`.
    ///
    /// The accepted range is `0..=`[`MAX_AUDIO_DELAY_MICROS`]
    /// (`upstream/UxPlay/lib/raop.c:759-761`); validating configuration input is
    /// the configuration layer's job, and this stores the value as given.
    pub fn set_audio_delay_micros(&mut self, micros: u32) {
        self.audio_delay_micros = micros;
    }

    /// Handles one complete request and decides what to write back.
    ///
    /// The steps follow `upstream/UxPlay/lib/raop.c:232-494`:
    ///
    /// 1. bound and parse the head, then bound the declared body and require it
    ///    to be present;
    /// 2. read `CSeq`, dropping the request when it is unusable
    ///    (`raop.c:247-250`);
    /// 3. drop a request without `CSeq` unless HLS support is on or the request
    ///    is a BLE beacon response (`raop.c:263-266`);
    /// 4. bind the connection to the first `X-Apple-Session-ID` it sees and
    ///    reject a later different one (`raop.c:400-402`);
    /// 5. route by method and URL, then add the headers upstream adds:
    ///    `Audio-Jack-Status`, the handler's headers, `Server` and `CSeq`.
    ///
    /// # Errors
    ///
    /// Returns [`ControlError`] when the request cannot be acted on; the caller
    /// answers with [`ControlError::to_response`], whose status code comes from
    /// [`ControlError::status`].
    pub fn handle(&mut self, input: &[u8]) -> Result<ControlOutcome, ControlError> {
        let head = parse_request_head_with_maximum(input, DEFAULT_MAX_HEAD).map_err(head_error)?;
        let length = head.content_length(MAX_CONTROL_BODY).map_err(body_error)?;
        // The body must be complete before the request is routed. Its contents
        // describe media sessions and belong to the network service work (W5).
        let _body = split_body(&input[head.consumed..], length)?;
        let facts = RequestFacts::from_head(&head);

        let cseq = match head.header("cseq") {
            None => None,
            Some(value) => match value.trim().parse::<u32>() {
                // Upstream drops a request whose CSeq is not a non-negative
                // integer instead of answering it (`raop.c:241-250`).
                Err(_) => return Ok(ControlOutcome::Ignored),
                Ok(cseq) => Some(cseq),
            },
        };

        if cseq.is_none() && !facts.is_ble_beacon_response() && !self.hls_support {
            return Ok(ControlOutcome::Ignored);
        }

        if let Some(id) = head.header("x-apple-session-id") {
            match self.session_id.as_deref() {
                None => {}
                Some(known) if known == id => {}
                Some(_) => return Err(ControlError::SessionMismatch),
            }
            if self.session_id.is_none() {
                self.session_id = Some(id.to_owned());
            }
        }

        let dispatch = dispatch(&facts);
        let (status, reason, ported) = match dispatch {
            Dispatch::Ported(route) => {
                self.observe(route);
                (200, "OK", Some(route))
            }
            Dispatch::NotImplemented(_) | Dispatch::UnknownRtspMethod => {
                (501, "Not Implemented", None)
            }
            Dispatch::Unhandled => (200, "OK", None),
        };

        let mut response = ControlResponse::new(&head.version, status, reason);
        // `raop.c:338-340`: added before the handler runs, for every RTSP
        // request that carries a CSeq except RECORD.
        if cseq.is_some() && head.method != "RECORD" {
            response
                .headers
                .push(("Audio-Jack-Status".to_owned(), AUDIO_JACK_STATUS.to_owned()));
        }
        // Handler headers (`raop_handlers.h:596`, `:1225-1226`, `:1284`).
        match ported {
            Some(Route::Options) => response
                .headers
                .push(("Public".to_owned(), PUBLIC_METHODS.to_owned())),
            Some(Route::Record) => {
                response.headers.push((
                    "Audio-Latency".to_owned(),
                    audio_latency_samples(self.audio_delay_micros).to_string(),
                ));
                response.headers.push((
                    "Audio-Jack-Status".to_owned(),
                    AUDIO_JACK_STATUS_RECORD.to_owned(),
                ));
            }
            Some(Route::Teardown) => {
                response
                    .headers
                    .push(("Connection".to_owned(), "close".to_owned()));
            }
            _ => {}
        }
        // `raop.c:488-492`: `Server` and `CSeq` come last, and an HLS request
        // gets neither.
        if !facts.is_hls_request() {
            response
                .headers
                .push(("Server".to_owned(), SERVER_HEADER_VALUE.to_owned()));
            if let Some(cseq) = cseq {
                response.headers.push(("CSeq".to_owned(), cseq.to_string()));
            }
        }
        Ok(ControlOutcome::Respond(response))
    }

    /// Records what a ported route means for the observed lifecycle state.
    ///
    /// The mapping is monotone and permissive, because upstream answers all of
    /// these routes with `200 OK` regardless of what came before
    /// (`upstream/UxPlay/lib/raop.c:406-478`, `raop_handlers.h:1215-1250`):
    /// `OPTIONS` never changes the state, a second `SETUP` does not rewind a
    /// recording session, and `FLUSH` outside a recording session is a no-op
    /// upstream only logs a warning about.
    fn observe(&mut self, route: Route) {
        self.state = match route {
            Route::Announce if self.state == ControlState::New => ControlState::Announced,
            Route::Setup if matches!(self.state, ControlState::New | ControlState::Announced) => {
                ControlState::Setup
            }
            Route::Record
                if matches!(self.state, ControlState::Setup | ControlState::Recording) =>
            {
                ControlState::Recording
            }
            Route::Teardown => ControlState::Closed,
            _ => self.state,
        };
    }
}

fn head_error(error: ParseError) -> ControlError {
    match error {
        ParseError::TooLarge { declared, maximum } => {
            ControlError::HeadTooLarge { declared, maximum }
        }
        other => ControlError::Parse(other),
    }
}

fn body_error(error: ParseError) -> ControlError {
    match error {
        ParseError::TooLarge { declared, maximum } => {
            ControlError::BodyTooLarge { declared, maximum }
        }
        other => ControlError::Parse(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, target: &str, cseq: Option<u32>, extra: &str, body: &[u8]) -> Vec<u8> {
        let mut head = format!("{method} {target} RTSP/1.0\r\n");
        if let Some(cseq) = cseq {
            head.push_str(&format!("CSeq: {cseq}\r\n"));
        }
        head.push_str(extra);
        head.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    #[track_caller]
    fn respond(session: &mut ControlSession, input: &[u8]) -> ControlResponse {
        match session.handle(input).unwrap() {
            ControlOutcome::Respond(response) => response,
            ControlOutcome::Ignored => panic!("expected a response, got Ignored"),
        }
    }

    #[track_caller]
    fn assert_header(response: &ControlResponse, name: &str, value: &str) {
        assert_eq!(
            response.header(name),
            Some(value),
            "header {name} in {:?}",
            response.headers
        );
    }

    #[test]
    fn lifecycle_is_observable_and_never_rejects_a_route() {
        let mut session = ControlSession::new();
        let options = respond(&mut session, &request("OPTIONS", "*", Some(1), "", &[]));
        assert_eq!(session.state(), ControlState::New);
        assert_header(&options, "Public", PUBLIC_METHODS);
        assert_header(&options, "Server", SERVER_HEADER_VALUE);
        assert_header(&options, "Audio-Jack-Status", AUDIO_JACK_STATUS);

        // SETUP works on a fresh connection: no ANNOUNCE is required, because
        // upstream has no ANNOUNCE route at all.
        let setup = respond(
            &mut session,
            &request("SETUP", "/stream", Some(2), "", b"plist"),
        );
        assert_eq!(setup.status, 200);
        assert_eq!(session.state(), ControlState::Setup);

        // OPTIONS stays valid in every state.
        respond(&mut session, &request("OPTIONS", "*", Some(3), "", &[]));
        assert_eq!(session.state(), ControlState::Setup);

        let record = respond(
            &mut session,
            &request("RECORD", "/stream", Some(4), "", &[]),
        );
        assert_eq!(session.state(), ControlState::Recording);
        assert_header(&record, "Audio-Latency", "11025");
        assert_header(&record, "Audio-Jack-Status", AUDIO_JACK_STATUS_RECORD);

        // FLUSH and a repeated SETUP keep the recording state.
        respond(&mut session, &request("FLUSH", "/stream", Some(5), "", &[]));
        assert_eq!(session.state(), ControlState::Recording);
        respond(
            &mut session,
            &request("SETUP", "/stream", Some(6), "", b"plist"),
        );
        assert_eq!(session.state(), ControlState::Recording);

        let teardown = respond(
            &mut session,
            &request("TEARDOWN", "/stream", Some(7), "", &[]),
        );
        assert_header(&teardown, "Connection", "close");
        assert_eq!(session.state(), ControlState::Closed);
    }

    #[test]
    fn announce_is_an_extension_that_only_marks_the_state() {
        let mut session = ControlSession::new();
        respond(
            &mut session,
            &request("ANNOUNCE", "/stream", Some(1), "", b"v=0\r\n"),
        );
        assert_eq!(session.state(), ControlState::Announced);
    }

    #[test]
    fn ignores_unusable_and_missing_cseq_like_upstream() {
        let mut session = ControlSession::new();
        // raop.c:247-250: a CSeq that is not a non-negative integer is dropped.
        assert_eq!(
            session
                .handle(b"OPTIONS * RTSP/1.0\r\nCSeq: x\r\n\r\n")
                .unwrap(),
            ControlOutcome::Ignored
        );
        assert_eq!(
            session
                .handle(b"OPTIONS * RTSP/1.0\r\nCSeq: -1\r\n\r\n")
                .unwrap(),
            ControlOutcome::Ignored
        );
        // raop.c:263-266: no CSeq, HLS support off, not a BLE beacon response.
        assert_eq!(
            session.handle(b"OPTIONS * RTSP/1.0\r\n\r\n").unwrap(),
            ControlOutcome::Ignored
        );
        assert_eq!(session.state(), ControlState::New);
    }

    #[test]
    fn ble_beacon_responses_and_hls_requests_take_their_own_paths() {
        let mut session = ControlSession::new();
        // raop.c:257-260: a BLE beacon response has no CSeq but is not dropped.
        let ble = respond(
            &mut session,
            b"GET /info?txtAirPlay RTSP/1.0\r\nContent-Length: 0\r\n\r\n",
        );
        assert_eq!(ble.status, 501);
        assert_header(&ble, "Server", SERVER_HEADER_VALUE);
        assert_eq!(ble.header("CSeq"), None);
        assert_eq!(ble.header("Audio-Jack-Status"), None);

        // raop.c:270 plus raop.c:263-266: without HLS support the same shape of
        // request with a Host header is dropped...
        let mut plain_session = ControlSession::new();
        assert_eq!(
            plain_session
                .handle(b"GET /hls/x.m3u8 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
                .unwrap(),
            ControlOutcome::Ignored
        );
        // ...and with it enabled the HLS route answers 501 with no Server or
        // CSeq header (raop.c:476-477, raop.c:488).
        session.set_hls_support(true);
        let hls = respond(
            &mut session,
            b"GET /hls/x.m3u8 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        );
        assert_eq!(hls.status, 501);
        assert_eq!(hls.header("Server"), None);
        assert_eq!(hls.header("CSeq"), None);
    }

    #[test]
    fn binds_the_connection_to_one_apple_session_id() {
        let mut session = ControlSession::new();
        let first = respond(
            &mut session,
            &request("OPTIONS", "*", Some(1), "X-Apple-Session-ID: abc\r\n", &[]),
        );
        assert_eq!(session.session_id(), Some("abc"));
        assert_eq!(
            first.header("Session"),
            None,
            "upstream never sends a Session header"
        );
        respond(
            &mut session,
            &request("OPTIONS", "*", Some(2), "X-Apple-Session-ID: abc\r\n", &[]),
        );
        assert_eq!(
            session.handle(&request(
                "OPTIONS",
                "*",
                Some(3),
                "X-Apple-Session-ID: other\r\n",
                &[]
            )),
            Err(ControlError::SessionMismatch)
        );
        assert_eq!(session.session_id(), Some("abc"));
    }

    #[test]
    fn echoes_the_request_protocol_and_frames_the_body_like_upstream() {
        let mut session = ControlSession::new();
        let response = respond(&mut session, &request("OPTIONS", "*", Some(7), "", &[]));
        let bytes = response.to_bytes();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("RTSP/1.0 200 OK\r\n"));
        // http_response.c:151-168: no body and no Content-Type means no
        // Content-Length at all.
        assert!(!text.contains("Content-Length"), "{text}");
        assert!(text.ends_with("\r\n\r\n"));

        let http = respond(
            &mut session,
            b"OPTIONS * HTTP/1.1\r\nCSeq: 8\r\nContent-Length: 0\r\n\r\n",
        );
        assert!(http.to_bytes().starts_with(b"HTTP/1.1 200 OK\r\n"));

        let mut with_type = ControlResponse::new("RTSP/1.0", 200, "OK");
        with_type
            .headers
            .push(("Content-Type".to_owned(), "text/parameters".to_owned()));
        assert_eq!(
            String::from_utf8(with_type.to_bytes()).unwrap(),
            "RTSP/1.0 200 OK\r\nContent-Type: text/parameters\r\nContent-Length: 0\r\n\r\n"
        );

        let mut with_body = ControlResponse::new("RTSP/1.0", 200, "OK");
        with_body.body = b"hi".to_vec();
        assert_eq!(
            String::from_utf8(with_body.to_bytes()).unwrap(),
            "RTSP/1.0 200 OK\r\nContent-Length: 2\r\n\r\nhi"
        );
    }

    #[test]
    fn answers_501_for_unported_routes_and_unknown_methods() {
        let mut session = ControlSession::new();
        for (method, target, version) in [
            ("POST", "/feedback", "RTSP/1.0"),
            ("GET", "/info", "RTSP/1.0"),
            ("PAUSE", "/stream", "RTSP/1.0"),
        ] {
            let input =
                format!("{method} {target} {version}\r\nCSeq: 1\r\nContent-Length: 0\r\n\r\n");
            let response = respond(&mut session, input.as_bytes());
            assert_eq!(response.status, 501, "{method} {target}");
            assert_eq!(response.reason, "Not Implemented");
            assert_header(&response, "Server", SERVER_HEADER_VALUE);
            assert_header(&response, "CSeq", "1");
        }
        // A route that is not implemented must not advance the session either.
        assert_eq!(session.state(), ControlState::New);
    }

    #[test]
    fn answers_upstream_default_for_unmatched_targets() {
        let mut session = ControlSession::new();
        let response = respond(&mut session, &request("POST", "/unknown", Some(1), "", &[]));
        assert_eq!(response.status, 200);
        assert_eq!(response.reason, "OK");
        assert!(response.body.is_empty());
        assert_header(&response, "Server", SERVER_HEADER_VALUE);
    }

    #[test]
    fn maps_errors_to_upstream_status_codes() {
        let parse = ControlError::Parse(ParseError::Invalid("header"));
        assert_eq!(parse.status(), (400, "Bad Request"));
        let head = ControlError::HeadTooLarge {
            declared: 70 * 1024,
            maximum: MAX_CONTROL_BODY,
        };
        assert_eq!(head.status(), (431, "Request Header Fields Too Large"));
        let body = ControlError::BodyTooLarge {
            declared: MAX_CONTROL_BODY + 1,
            maximum: MAX_CONTROL_BODY,
        };
        assert_eq!(body.status(), (400, "Bad Request"));
        assert_eq!(ControlError::SessionMismatch.status(), (400, "Bad Request"));
        // raop.c:236-238 returns before any header is added.
        assert_eq!(
            head.to_response("RTSP/1.0").to_bytes(),
            b"RTSP/1.0 431 Request Header Fields Too Large\r\n\r\n"
        );
    }

    #[test]
    fn rejects_oversized_and_incomplete_requests() {
        let mut session = ControlSession::new();
        let oversized_header = format!(
            "SETUP /stream RTSP/1.0\r\nX: {}\r\n\r\n",
            "a".repeat(70 * 1024)
        );
        assert!(matches!(
            session.handle(oversized_header.as_bytes()),
            Err(ControlError::HeadTooLarge { .. })
        ));

        let oversized_body = format!(
            "SETUP /stream RTSP/1.0\r\nCSeq: 1\r\nContent-Length: {}\r\n\r\n",
            MAX_CONTROL_BODY + 1
        );
        assert!(matches!(
            session.handle(oversized_body.as_bytes()),
            Err(ControlError::BodyTooLarge { .. })
        ));

        // A declared body that never arrived is a parse failure, not a request
        // to act on.
        let truncated = b"SETUP /stream RTSP/1.0\r\nCSeq: 1\r\nContent-Length: 4\r\n\r\nda";
        assert!(matches!(
            session.handle(truncated),
            Err(ControlError::Parse(ParseError::Incomplete))
        ));

        let malformed = b"GET / FTP/1.0\r\n\r\n";
        assert!(matches!(
            session.handle(malformed),
            Err(ControlError::Parse(ParseError::Invalid("start line")))
        ));
    }

    #[test]
    fn audio_latency_follows_the_upstream_formula() {
        assert_eq!(audio_latency_samples(DEFAULT_AUDIO_DELAY_MICROS), 11_025);
        assert_eq!(audio_latency_samples(0), 0);
        assert_eq!(audio_latency_samples(MAX_AUDIO_DELAY_MICROS), 441_000);
        // The narrowest possible input still fits: even a saturated delay times
        // the sample rate stays below `u32::MAX`.
        assert_eq!(audio_latency_samples(u32::MAX), 189_408_057);
    }

    #[test]
    fn record_reports_the_configured_audio_delay() {
        let mut session = ControlSession::new();
        session.set_audio_delay_micros(500_000);
        respond(
            &mut session,
            &request("SETUP", "/stream", Some(1), "", b"plist"),
        );
        let record = respond(
            &mut session,
            &request("RECORD", "/stream", Some(2), "", &[]),
        );
        assert_header(&record, "Audio-Latency", "22050");
    }
}
