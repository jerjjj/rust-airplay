// SPDX-License-Identifier: GPL-3.0-or-later
//! Upstream control-plane specification.
//!
//! These tables pin the observable control-plane contract to
//! `upstream/UxPlay@57ea83411d5f7e0b38c5841987439340543f025c`:
//!
//! - route selection: `upstream/UxPlay/lib/raop.c:406-478`, plus the HLS rule at
//!   `upstream/UxPlay/lib/raop.c:270`;
//! - `Public` header: `upstream/UxPlay/lib/raop_handlers.h:596`;
//! - response headers: `upstream/UxPlay/lib/raop.c:338-340`,
//!   `upstream/UxPlay/lib/raop.c:488-492`,
//!   `upstream/UxPlay/lib/raop_handlers.h:1225-1226`;
//! - status codes: `upstream/UxPlay/lib/raop.c:335` (default `200`),
//!   `upstream/UxPlay/lib/raop.c:442-444` and
//!   `upstream/UxPlay/lib/raop_handlers.h:558` (`501`),
//!   `upstream/UxPlay/lib/http_handlers.h:657` and `:891` (`400`),
//!   `upstream/UxPlay/lib/http_handlers.h:910`, `:926` and `:974` (`404`).
//!
//! Known divergences that the routing/session rework must close (see
//! `docs/S3_WORK_PLAN.md`, W3). They are listed here instead of being asserted
//! as correct behavior:
//!
//! - upstream tolerates a missing `CSeq` (it only omits the `CSeq` and
//!   `Audio-Jack-Status` headers), while this port still requires it;
//! - upstream accepts `SETUP` on a fresh connection and never requires
//!   `ANNOUNCE`, while the port's state machine still rejects both;
//! - upstream keys the session on the client's `X-Apple-Session-ID`
//!   (`upstream/UxPlay/lib/raop.c:268-297`), while the port compares a constant;
//! - `ControlResponse::to_bytes` always writes `RTSP/1.0`, while upstream echoes
//!   the request protocol for HTTP/1.1 peers;
//! - `HLS` and every unported upstream route answer `501` here, while upstream
//!   serves them (or ignores a CSeq-less request unless `-hls` is set).

use airplay_core::control::{
    AUDIO_JACK_STATUS, AUDIO_JACK_STATUS_RECORD, ControlResponse, ControlSession, Dispatch,
    PUBLIC_METHODS, RequestFacts, Route, SERVER_HEADER_VALUE, dispatch,
};
use airplay_core::wire::parse_request_head;

fn facts<'a>(method: &'a str, target: &'a str, version: &'a str) -> RequestFacts<'a> {
    RequestFacts {
        method,
        target,
        version,
        has_cseq: true,
        has_host: false,
        has_session_id: false,
    }
}

fn request(method: &str, target: &str, version: &str, cseq: u32, extra: &str) -> Vec<u8> {
    format!("{method} {target} {version}\r\nCSeq: {cseq}\r\n{extra}Content-Length: 0\r\n\r\n")
        .into_bytes()
}

fn with_body(method: &str, target: &str, version: &str, cseq: u32, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "{method} {target} {version}\r\nCSeq: {cseq}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

#[track_caller]
fn assert_header(response: &ControlResponse, name: &str, value: &str) {
    assert!(
        response
            .headers
            .iter()
            .any(|(header, found)| header == name && found == value),
        "missing header {name}: {value} in {:?}",
        response.headers
    );
}

#[test]
fn route_table_matches_upstream() {
    use Dispatch::{NotImplemented, Ported, Unhandled, UnknownRtspMethod};
    let cases: &[(&str, &str, &str, Dispatch)] = &[
        // RTSP/1.0 `POST` handlers (raop.c:407-422).
        (
            "POST",
            "/feedback",
            "RTSP/1.0",
            NotImplemented(Route::Feedback),
        ),
        (
            "POST",
            "/pair-pin-start",
            "RTSP/1.0",
            NotImplemented(Route::PairPinStart),
        ),
        (
            "POST",
            "/pair-setup-pin",
            "RTSP/1.0",
            NotImplemented(Route::PairSetupPin),
        ),
        (
            "POST",
            "/pair-setup",
            "RTSP/1.0",
            NotImplemented(Route::PairSetup),
        ),
        (
            "POST",
            "/pair-verify",
            "RTSP/1.0",
            NotImplemented(Route::PairVerify),
        ),
        (
            "POST",
            "/fp-setup",
            "RTSP/1.0",
            NotImplemented(Route::FairPlaySetup),
        ),
        (
            "POST",
            "/audioMode",
            "RTSP/1.0",
            NotImplemented(Route::AudioMode),
        ),
        // RTSP/1.0 `GET`: upstream matches the URL with `strstr` (raop.c:424).
        ("GET", "/info", "RTSP/1.0", NotImplemented(Route::Info)),
        ("GET", "/info?x=1", "RTSP/1.0", NotImplemented(Route::Info)),
        ("GET", "/other", "RTSP/1.0", Unhandled),
        // RTSP/1.0 method-only handlers (raop.c:427-441).
        ("OPTIONS", "*", "RTSP/1.0", Ported(Route::Options)),
        (
            "SETUP",
            "rtsp://host/stream",
            "RTSP/1.0",
            Ported(Route::Setup),
        ),
        (
            "GET_PARAMETER",
            "/stream",
            "RTSP/1.0",
            NotImplemented(Route::GetParameter),
        ),
        (
            "SET_PARAMETER",
            "/stream",
            "RTSP/1.0",
            NotImplemented(Route::SetParameter),
        ),
        ("RECORD", "/stream", "RTSP/1.0", Ported(Route::Record)),
        ("FLUSH", "/stream", "RTSP/1.0", Ported(Route::Flush)),
        ("TEARDOWN", "/stream", "RTSP/1.0", Ported(Route::Teardown)),
        // Documented extension: upstream has no ANNOUNCE route at all.
        ("ANNOUNCE", "/stream", "RTSP/1.0", Ported(Route::Announce)),
        // Unknown RTSP method: raop.c:442-444 answers 501.
        ("PAUSE", "/stream", "RTSP/1.0", UnknownRtspMethod),
        // HTTP/1.1 POST handlers (raop.c:446-463).
        (
            "POST",
            "/reverse",
            "HTTP/1.1",
            NotImplemented(Route::Reverse),
        ),
        ("POST", "/play", "HTTP/1.1", NotImplemented(Route::Play)),
        (
            "POST",
            "/getProperty?key=x",
            "HTTP/1.1",
            NotImplemented(Route::GetProperty),
        ),
        (
            "POST",
            "/scrub?position=1",
            "HTTP/1.1",
            NotImplemented(Route::Scrub),
        ),
        (
            "POST",
            "/rate?value=1",
            "HTTP/1.1",
            NotImplemented(Route::Rate),
        ),
        ("POST", "/stop", "HTTP/1.1", NotImplemented(Route::Stop)),
        ("POST", "/action", "HTTP/1.1", NotImplemented(Route::Action)),
        (
            "POST",
            "/fp-setup2",
            "HTTP/1.1",
            NotImplemented(Route::FairPlaySetup2),
        ),
        // HTTP/1.1 GET and PUT handlers (raop.c:464-475).
        (
            "GET",
            "/server-info",
            "HTTP/1.1",
            NotImplemented(Route::ServerInfo),
        ),
        (
            "GET",
            "/playback-info",
            "HTTP/1.1",
            NotImplemented(Route::PlaybackInfo),
        ),
        (
            "PUT",
            "/setProperty?x=1",
            "HTTP/1.1",
            NotImplemented(Route::SetProperty),
        ),
        // No handler selected: upstream keeps its default 200 (raop.c:334-335).
        ("POST", "/unknown", "HTTP/1.1", Unhandled),
        ("PATCH", "/reverse", "HTTP/1.1", Unhandled),
        // The HTTP/1.1 table has no `/info` route.
        ("GET", "/info", "HTTP/1.1", Unhandled),
        // Upstream compares the protocol string exactly (raop.c:406, 445).
        ("POST", "/feedback", "RTSP/2.0", Unhandled),
    ];
    for (method, target, version, expected) in cases {
        assert_eq!(
            dispatch(&facts(method, target, version)),
            *expected,
            "{method} {target} {version}"
        );
    }
}

#[test]
fn hls_rule_precedes_both_protocol_tables() {
    let hls = RequestFacts {
        method: "GET",
        target: "/hls/playlist.m3u8",
        version: "HTTP/1.1",
        has_cseq: false,
        has_host: true,
        has_session_id: false,
    };
    assert_eq!(dispatch(&hls), Dispatch::NotImplemented(Route::Hls));

    // CSeq or an Apple session id keeps the request on the normal tables.
    let with_cseq = RequestFacts {
        has_cseq: true,
        ..hls
    };
    assert_eq!(dispatch(&with_cseq), Dispatch::Unhandled);
    let with_session = RequestFacts {
        has_session_id: true,
        ..hls
    };
    assert_eq!(dispatch(&with_session), Dispatch::Unhandled);
}

#[test]
fn request_facts_come_from_the_parsed_head() {
    let head = parse_request_head(
        b"GET /info RTSP/1.0\r\nCSeq: 1\r\nX-Apple-Session-ID: abc\r\nContent-Length: 0\r\n\r\n",
    )
    .unwrap();
    let facts = RequestFacts::from_head(&head);
    assert!(facts.has_cseq && facts.has_session_id);
    assert!(!facts.has_host);
    assert_eq!(dispatch(&facts), Dispatch::NotImplemented(Route::Info));

    let hls = parse_request_head(b"GET /x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
    assert_eq!(
        dispatch(&RequestFacts::from_head(&hls)),
        Dispatch::NotImplemented(Route::Hls)
    );
}

#[test]
fn announce_is_an_unadvertised_extension() {
    assert!(!PUBLIC_METHODS.contains("ANNOUNCE"));
    assert_eq!(
        dispatch(&facts("ANNOUNCE", "/stream", "RTSP/1.0")),
        Dispatch::Ported(Route::Announce)
    );
}

#[test]
fn options_response_pins_public_and_upstream_headers() {
    let mut session = ControlSession::new();
    let response = session
        .handle(&request("OPTIONS", "*", "RTSP/1.0", 7, ""))
        .unwrap();
    assert_eq!(response.status, 200);
    assert_header(&response, "CSeq", "7");
    assert_header(&response, "Server", SERVER_HEADER_VALUE);
    assert_header(&response, "Audio-Jack-Status", AUDIO_JACK_STATUS);
    assert_header(&response, "Public", PUBLIC_METHODS);
}

#[test]
fn record_response_uses_the_analog_jack_status() {
    let mut session = ControlSession::new();
    session
        .handle(&with_body("ANNOUNCE", "/stream", "RTSP/1.0", 1, b"v=0\r\n"))
        .unwrap();
    session
        .handle(&with_body("SETUP", "/stream", "RTSP/1.0", 2, b"tp"))
        .unwrap();
    let response = session
        .handle(&request(
            "RECORD",
            "/stream",
            "RTSP/1.0",
            3,
            "Session: rust-airplay\r\n",
        ))
        .unwrap();
    assert_eq!(response.status, 200);
    assert_header(&response, "Audio-Jack-Status", AUDIO_JACK_STATUS_RECORD);
    assert!(
        !response
            .headers
            .iter()
            .any(|(_, value)| value == AUDIO_JACK_STATUS)
    );
}

#[test]
fn unported_and_unknown_routes_answer_501() {
    let mut session = ControlSession::new();
    for (method, target) in [
        ("POST", "/feedback"),
        ("GET", "/info"),
        ("PAUSE", "/stream"),
    ] {
        let response = session
            .handle(&request(method, target, "RTSP/1.0", 1, ""))
            .unwrap();
        assert_eq!(response.status, 501, "{method} {target}");
        assert_eq!(response.reason, "Not Implemented");
        assert_header(&response, "Server", SERVER_HEADER_VALUE);
    }
    // A route that is not implemented must not advance the session either.
    assert_eq!(session.state(), airplay_core::control::ControlState::New);
}
