// SPDX-License-Identifier: GPL-3.0-or-later
//! Route handlers for the network control service.
//!
//! Each handler is a port of one function in
//! `upstream/UxPlay/lib/raop_handlers.h`. A handler contributes only the status
//! code, reason phrase, body and headers its upstream counterpart adds: the
//! routing decision, the `Audio-Jack-Status`/`Server`/`CSeq` envelope and the
//! status of a route this port has not ported yet stay in [`crate::control`].

use crate::control::{Route, RouteHandler, RouteReply, RouteRequest};
use crate::fairplay::{FairPlaySession, HANDSHAKE_REQUEST_LEN, SETUP_REQUEST_LEN};
use crate::plist::{self, Plist};

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
    /// Curve25519 public key for pairing, sent as the `pk` data value.
    /// Upstream reads it from its persistent key file
    /// (`upstream/UxPlay/lib/raop_handlers.h:126-131`); pairing itself has not
    /// been ported, so a caller supplies it.
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
/// connection: the FairPlay session established by `POST /fp-setup` belongs to
/// the client that opened it.
pub struct RouteHandlers {
    identity: DeviceIdentity,
    fairplay: FairPlaySession,
    feedback: Option<Box<dyn FnMut() + Send>>,
}

impl RouteHandlers {
    /// Creates handlers that advertise `identity`.
    pub fn new(identity: DeviceIdentity) -> Self {
        Self {
            identity,
            fairplay: FairPlaySession::new(),
            feedback: None,
        }
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
            _ => None,
        }
    }
}

impl RouteHandlers {
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
