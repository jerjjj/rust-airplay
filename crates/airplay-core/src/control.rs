// SPDX-License-Identifier: GPL-3.0-or-later
//! RTSP/HTTP control-plane routing and session state.
//!
//! [`dispatch`] mirrors the method+URL table of
//! `upstream/UxPlay/lib/raop.c:406-478` and is the compatibility specification
//! surface; [`ControlSession`] adds the per-connection state that upstream keeps
//! in `raop_conn_t`. Socket I/O and service discovery stay outside this module.

#![allow(missing_docs)]

use std::fmt;

use crate::wire::{MessageHead, ParseError, parse_request_head, split_body};

const MAX_CONTROL_BODY: usize = 64 * 1024;
const SESSION_ID: &str = "rust-airplay";

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
    /// Internal HLS request: a `Host` header without `CSeq` and without
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
    /// Whether this port already implements the route.
    ///
    /// An unported route answers `501 Not Implemented` rather than the upstream
    /// handler's `200 OK`, so an unimplemented handler can never be mistaken for
    /// success.
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
    pub method: &'a str,
    pub target: &'a str,
    pub version: &'a str,
    pub has_cseq: bool,
    pub has_host: bool,
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
/// version selects no handler.
pub fn dispatch(facts: &RequestFacts<'_>) -> Dispatch {
    // `upstream/UxPlay/lib/raop.c:270`: the HLS branch is chosen before either
    // protocol table when a Host header arrives without CSeq or session id.
    if facts.has_host && !facts.has_cseq && !facts.has_session_id {
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
        // default 200 response untouched.
        _ => return Dispatch::Unhandled,
    };
    classify(route)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ControlState {
    #[default]
    New,
    Announced,
    Setup,
    Recording,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlResponse {
    pub status: u16,
    pub reason: &'static str,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl ControlResponse {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut output = Vec::with_capacity(128 + self.body.len());
        output
            .extend_from_slice(format!("RTSP/1.0 {} {}\r\n", self.status, self.reason).as_bytes());
        for (name, value) in &self.headers {
            output.extend_from_slice(name.as_bytes());
            output.extend_from_slice(b": ");
            output.extend_from_slice(value.as_bytes());
            output.extend_from_slice(b"\r\n");
        }
        output.extend_from_slice(format!("Content-Length: {}\r\n\r\n", self.body.len()).as_bytes());
        output.extend_from_slice(&self.body);
        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    Parse(ParseError),
    MissingHeader(&'static str),
    InvalidHeader(&'static str),
    InvalidState { route: Route, state: ControlState },
    SessionMismatch,
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(f),
            Self::MissingHeader(name) => write!(f, "missing RTSP header {name}"),
            Self::InvalidHeader(name) => write!(f, "invalid RTSP header {name}"),
            Self::InvalidState { route, state } => {
                write!(f, "RTSP route {route:?} is invalid in state {state:?}")
            }
            Self::SessionMismatch => f.write_str("RTSP Session header does not match connection"),
        }
    }
}

impl std::error::Error for ControlError {}

impl From<ParseError> for ControlError {
    fn from(error: ParseError) -> Self {
        Self::Parse(error)
    }
}

#[derive(Debug, Default)]
pub struct ControlSession {
    state: ControlState,
}

impl ControlSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> ControlState {
        self.state
    }

    /// Handles one complete request and produces its response.
    ///
    /// Ported routes run the session state machine; upstream routes that are not
    /// ported yet answer `501 Not Implemented`; a request that selects no handler
    /// answers upstream's default `200 OK` with an empty body.
    pub fn handle(&mut self, input: &[u8]) -> Result<ControlResponse, ControlError> {
        let head = parse_request_head(input)?;
        let length = head.content_length(MAX_CONTROL_BODY)?;
        let body = split_body(&input[head.consumed..], length)?;
        let cseq = required_u32(&head, "cseq")?;
        match dispatch(&RequestFacts::from_head(&head)) {
            Dispatch::Ported(route) => self.handle_ported(&head, body, cseq, route),
            Dispatch::NotImplemented(_) | Dispatch::UnknownRtspMethod => {
                Ok(default_response(501, "Not Implemented", cseq, &head.method))
            }
            Dispatch::Unhandled => Ok(default_response(200, "OK", cseq, &head.method)),
        }
    }

    fn handle_ported(
        &mut self,
        head: &MessageHead,
        body: &[u8],
        cseq: u32,
        route: Route,
    ) -> Result<ControlResponse, ControlError> {
        if matches!(self.state, ControlState::Setup | ControlState::Recording)
            && !matches!(route, Route::Options | Route::Setup)
        {
            let session = head
                .header("session")
                .ok_or(ControlError::MissingHeader("Session"))?;
            if session != SESSION_ID {
                return Err(ControlError::SessionMismatch);
            }
        }
        self.validate_body(head, body)?;
        self.advance(route)?;

        let mut response = default_response(200, "OK", cseq, &head.method);
        if matches!(self.state, ControlState::Setup | ControlState::Recording) {
            response
                .headers
                .push(("Session".to_owned(), SESSION_ID.to_owned()));
        }
        if route == Route::Record {
            // `upstream/UxPlay/lib/raop_handlers.h:1225-1226` also sends
            // `Audio-Latency`, which needs the configured audio delay and lands
            // with the configuration work.
            response.headers.push((
                "Audio-Jack-Status".to_owned(),
                AUDIO_JACK_STATUS_RECORD.to_owned(),
            ));
        }
        if route == Route::Options {
            response
                .headers
                .push(("Public".to_owned(), PUBLIC_METHODS.to_owned()));
        }
        Ok(response)
    }

    fn validate_body(&self, head: &MessageHead, body: &[u8]) -> Result<(), ControlError> {
        if matches!(head.method.as_str(), "ANNOUNCE" | "SETUP") && body.is_empty() {
            return Err(ControlError::InvalidHeader("Content-Length"));
        }
        Ok(())
    }

    fn advance(&mut self, route: Route) -> Result<(), ControlError> {
        let next = match (self.state, route) {
            (ControlState::New, Route::Options) => ControlState::New,
            (ControlState::New, Route::Announce) => ControlState::Announced,
            (ControlState::Announced, Route::Setup) => ControlState::Setup,
            (ControlState::Setup, Route::Record) => ControlState::Recording,
            (ControlState::Recording, Route::Flush) => ControlState::Recording,
            (ControlState::Setup | ControlState::Recording, Route::Teardown) => {
                ControlState::Closed
            }
            (state, route) => {
                return Err(ControlError::InvalidState { route, state });
            }
        };
        self.state = next;
        Ok(())
    }
}

/// Builds a response with the headers upstream attaches to every non-HLS reply.
fn default_response(status: u16, reason: &'static str, cseq: u32, method: &str) -> ControlResponse {
    let mut headers = Vec::new();
    if method != "RECORD" {
        headers.push(("Audio-Jack-Status".to_owned(), AUDIO_JACK_STATUS.to_owned()));
    }
    headers.push(("Server".to_owned(), SERVER_HEADER_VALUE.to_owned()));
    headers.push(("CSeq".to_owned(), cseq.to_string()));
    ControlResponse {
        status,
        reason,
        headers,
        body: Vec::new(),
    }
}

fn required_u32(head: &MessageHead, name: &'static str) -> Result<u32, ControlError> {
    head.header(name)
        .ok_or(ControlError::MissingHeader(name))?
        .parse()
        .map_err(|_| ControlError::InvalidHeader(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, target: &str, cseq: u32, extra: &str, body: &[u8]) -> Vec<u8> {
        format!(
            "{method} {target} RTSP/1.0\r\nCSeq: {cseq}\r\n{extra}Content-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes()
        .into_iter()
        .chain(body.iter().copied())
        .collect()
    }

    #[test]
    fn follows_control_lifecycle_and_echoes_headers() {
        let mut session = ControlSession::new();
        let options = session
            .handle(&request("OPTIONS", "/stream", 1, "", &[]))
            .unwrap();
        assert_eq!(session.state(), ControlState::New);
        let text = String::from_utf8(options.to_bytes()).unwrap();
        assert!(text.contains(&format!("Public: {PUBLIC_METHODS}\r\n")));
        assert!(text.contains(&format!("Server: {SERVER_HEADER_VALUE}\r\n")));
        assert!(text.contains(&format!("Audio-Jack-Status: {AUDIO_JACK_STATUS}\r\n")));
        session
            .handle(&request("ANNOUNCE", "/stream", 2, "", b"v=0\r\n"))
            .unwrap();
        session
            .handle(&request("SETUP", "/stream", 3, "", b"transport"))
            .unwrap();
        let response = session
            .handle(&request(
                "RECORD",
                "/stream",
                4,
                "Session: rust-airplay\r\n",
                &[],
            ))
            .unwrap();
        assert_eq!(session.state(), ControlState::Recording);
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name == "Session" && value == SESSION_ID)
        );
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name == "Audio-Jack-Status"
                    && value == AUDIO_JACK_STATUS_RECORD)
        );
        assert!(
            !response
                .headers
                .iter()
                .any(|(_, value)| value == AUDIO_JACK_STATUS)
        );
        session
            .handle(&request(
                "FLUSH",
                "/stream",
                5,
                "Session: rust-airplay\r\n",
                &[],
            ))
            .unwrap();
        session
            .handle(&request(
                "TEARDOWN",
                "/stream",
                6,
                "Session: rust-airplay\r\n",
                &[],
            ))
            .unwrap();
        assert_eq!(session.state(), ControlState::Closed);
    }

    #[test]
    fn rejects_bad_sequence_and_session() {
        let mut session = ControlSession::new();
        assert!(matches!(
            session.handle(b"OPTIONS / RTSP/1.0\r\n\r\n"),
            Err(ControlError::MissingHeader("cseq"))
        ));
        session
            .handle(&request("ANNOUNCE", "/stream", 1, "", b"x"))
            .unwrap();
        session
            .handle(&request("SETUP", "/stream", 2, "", b"x"))
            .unwrap();
        assert!(matches!(
            session.handle(&request("RECORD", "/stream", 3, "Session: wrong\r\n", &[])),
            Err(ControlError::SessionMismatch)
        ));
    }

    #[test]
    fn answers_501_for_unported_routes_and_unknown_methods() {
        let mut session = ControlSession::new();
        let unported = session
            .handle(&request("POST", "/feedback", 1, "", &[]))
            .unwrap();
        assert_eq!(unported.status, 501);
        assert_eq!(unported.reason, "Not Implemented");
        assert!(
            unported
                .headers
                .iter()
                .any(|(name, value)| name == "Server" && value == SERVER_HEADER_VALUE)
        );
        let unknown = session
            .handle(&request("PAUSE", "/stream", 2, "", &[]))
            .unwrap();
        assert_eq!(unknown.status, 501);
        assert_eq!(session.state(), ControlState::New);
    }

    #[test]
    fn answers_upstream_default_for_unmatched_targets() {
        let mut session = ControlSession::new();
        let response = session
            .handle(&request("POST", "/unknown", 1, "", &[]))
            .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.reason, "OK");
        assert!(response.body.is_empty());
    }

    #[test]
    fn rejects_oversized_declared_body() {
        let mut session = ControlSession::new();
        let request = format!(
            "SETUP /stream RTSP/1.0\r\nCSeq: 1\r\nContent-Length: {}\r\n\r\n",
            MAX_CONTROL_BODY + 1
        );
        assert!(matches!(
            session.handle(request.as_bytes()),
            Err(ControlError::Parse(ParseError::TooLarge { .. }))
        ));
    }
}
