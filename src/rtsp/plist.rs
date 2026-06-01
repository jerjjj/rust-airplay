use crate::config::Config;
pub use ::plist::Value;
pub use ::plist::Dictionary;
pub const APPLE_BPLIST: &str = "application/x-apple-binary-plist";

macro_rules! int {
    ($v:expr) => { Value::Integer(($v as i64).into()) };
}

pub fn build_info_plist(config: &Config) -> Vec<u8> {
    let mut r = Dictionary::new();
    r.insert("features".into(), int!(0x5A7FFFF7u64));
    r.insert("model".into(), Value::String(config.server_name.clone()));
    r.insert("sourceVersion".into(), Value::String("220.68".into()));
    r.insert("vv".into(), int!(2));
    r.insert("deviceID".into(), Value::String(config.mac_address.clone()));
    r.insert("statusFlags".into(), int!(68));
    r.insert("keepAliveSendStatsAsBody".into(), int!(1));

    let mut a0 = Dictionary::new();
    a0.insert("type".into(), int!(100)); a0.insert("audioInputFormats".into(), int!(67108860)); a0.insert("audioOutputFormats".into(), int!(67108860));
    let mut a1 = Dictionary::new();
    a1.insert("type".into(), int!(101)); a1.insert("audioInputFormats".into(), int!(67108860)); a1.insert("audioOutputFormats".into(), int!(67108860));
    r.insert("audioFormats".into(), Value::Array(vec![Value::Dictionary(a0), Value::Dictionary(a1)]));

    let mut l0 = Dictionary::new();
    l0.insert("type".into(), int!(100)); l0.insert("audioType".into(), Value::String("default".into())); l0.insert("inputLatencyMicros".into(), Value::Boolean(false));
    let mut l1 = Dictionary::new();
    l1.insert("type".into(), int!(101)); l1.insert("audioType".into(), Value::String("default".into())); l1.insert("inputLatencyMicros".into(), Value::Boolean(false));
    r.insert("audioLatencies".into(), Value::Array(vec![Value::Dictionary(l0), Value::Dictionary(l1)]));

    let mut d = Dictionary::new();
    d.insert("features".into(), int!(14));
    d.insert("width".into(), int!(config.width)); d.insert("height".into(), int!(config.height));
    d.insert("widthPixels".into(), int!(config.width)); d.insert("heightPixels".into(), int!(config.height));
    d.insert("widthPhysical".into(), Value::Boolean(false)); d.insert("heightPhysical".into(), Value::Boolean(false));
    d.insert("maxFPS".into(), int!(config.fps)); d.insert("overscanned".into(), Value::Boolean(false));
    d.insert("refreshRate".into(), int!(config.refresh_rate)); d.insert("rotation".into(), Value::Boolean(false));
    d.insert("uuid".into(), Value::String("e5f7a68d-7b0f-4305-984b-974f677a150b".into()));
    r.insert("displays".into(), Value::Array(vec![Value::Dictionary(d)]));
    r.insert("pi".into(), Value::String("b08f5a79-db29-4384-b456-a4784d9e6055".into()));
    r.insert("name".into(), Value::String(config.server_name.clone()));
    let mut buf = Vec::new();
    ::plist::to_writer_binary(&mut buf, &Value::Dictionary(r)).unwrap();
    buf
}

pub fn build_video_setup_response(dp: u16, ep: u16) -> Vec<u8> {
    let mut r = Dictionary::new();
    let mut s = Dictionary::new();
    s.insert("dataPort".into(), int!(dp)); s.insert("type".into(), int!(110));
    r.insert("streams".into(), Value::Array(vec![Value::Dictionary(s)]));
    r.insert("eventPort".into(), int!(ep)); r.insert("timingPort".into(), int!(0));
    let mut buf = Vec::new();
    ::plist::to_writer_binary(&mut buf, &Value::Dictionary(r)).unwrap();
    buf
}

pub fn build_audio_setup_response(dp: u16, cp: u16) -> Vec<u8> {
    let mut r = Dictionary::new();
    let mut s = Dictionary::new();
    s.insert("dataPort".into(), int!(dp)); s.insert("type".into(), int!(96)); s.insert("controlPort".into(), int!(cp));
    r.insert("streams".into(), Value::Array(vec![Value::Dictionary(s)]));
    let mut buf = Vec::new();
    ::plist::to_writer_binary(&mut buf, &Value::Dictionary(r)).unwrap();
    buf
}
