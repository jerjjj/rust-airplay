// SPDX-License-Identifier: GPL-3.0-or-later
//! Tests for the route handlers.
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
    handlers.info(&RouteRequest {
        head,
        body,
        peer: None,
    })
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
        peer: None,
    });
    // Version byte 4 is 0, which is not the FairPlay version upstream accepts.
    assert_eq!(rejected.status, 501);

    let (head, body) = parse("POST /fp-setup", "CSeq: 1\r\n", &[0; 5]);
    let ignored = handlers.fairplay_setup(&RouteRequest {
        head: &head,
        body: &body,
        peer: None,
    });
    assert_eq!(ignored.status, 200);
    assert!(ignored.body.is_empty());
}
