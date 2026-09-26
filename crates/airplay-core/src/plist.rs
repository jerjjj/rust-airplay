// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded property-list access for AirPlay control payloads.
//!
//! UxPlay speaks the binary and XML property-list formats through libplist:
//! `upstream/UxPlay/lib/raop.c:370` parses SETUP requests, `raop.c:518` parses
//! `/pair-verify` requests, and responses are written with `plist_to_bin`
//! (`upstream/UxPlay/lib/raop_handlers.h:246`). This module keeps the same
//! observable behavior behind two rules the rest of the port depends on:
//!
//! - the payload size is checked before the parser runs;
//! - typed access returns a typed error instead of panicking, so a malformed or
//!   truncated request body can never abort the process.
//!
//! Values are handed out as [`plist::Value`] on purpose: navigation of deeper
//! structures (for example `streams[]`) needs the full value API, and this port
//! has no reason to hide it.

use std::io::Cursor;

/// Largest accepted plist payload. Matches the control-plane body bound in
/// [`crate::control`]; real AirPlay plists are a few kilobytes at most.
pub const MAX_PLIST_BYTES: usize = 64 * 1024;

/// Reasons a property list could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlistError {
    /// The payload is larger than the configured bound and was not parsed.
    TooLarge {
        /// Size of the rejected payload.
        declared: usize,
        /// Configured bound.
        maximum: usize,
    },
    /// The payload is not a property list the parser accepted.
    Decode(String),
    /// A value could not be serialized.
    Encode(String),
    /// A required key is absent.
    MissingKey(&'static str),
    /// A key exists with a different value type.
    WrongType {
        /// Key whose value has the unexpected type.
        key: &'static str,
        /// Type the caller expected.
        expected: &'static str,
    },
}

impl std::fmt::Display for PlistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { declared, maximum } => {
                write!(f, "plist size {declared} exceeds maximum {maximum}")
            }
            Self::Decode(reason) => write!(f, "invalid plist: {reason}"),
            Self::Encode(reason) => write!(f, "cannot encode plist: {reason}"),
            Self::MissingKey(key) => write!(f, "missing plist key {key}"),
            Self::WrongType { key, expected } => {
                write!(f, "plist key {key} is not a {expected}")
            }
        }
    }
}

impl std::error::Error for PlistError {}

/// A parsed property list.
#[derive(Debug, Clone, PartialEq)]
pub struct Plist {
    value: plist::Value,
}

impl Plist {
    /// Parses a binary or XML property list, bounded by [`MAX_PLIST_BYTES`].
    pub fn parse(bytes: &[u8]) -> Result<Self, PlistError> {
        Self::parse_with_maximum(bytes, MAX_PLIST_BYTES)
    }

    /// Parses a binary or XML property list with an explicit size bound.
    ///
    /// The bound is checked before the parser runs, so an oversized request
    /// cannot drive the allocation of a large value tree.
    pub fn parse_with_maximum(bytes: &[u8], maximum: usize) -> Result<Self, PlistError> {
        if bytes.len() > maximum {
            return Err(PlistError::TooLarge {
                declared: bytes.len(),
                maximum,
            });
        }
        let value = plist::Value::from_reader(Cursor::new(bytes))
            .map_err(|error| PlistError::Decode(error.to_string()))?;
        Ok(Self { value })
    }

    /// Wraps an already built value, for example a response under construction.
    pub fn from_value(value: plist::Value) -> Self {
        Self { value }
    }

    /// Borrows the wrapped value.
    pub fn value(&self) -> &plist::Value {
        &self.value
    }

    /// Unwraps the value.
    pub fn into_value(self) -> plist::Value {
        self.value
    }

    /// Borrows the root dictionary, or `None` when the root is not a dictionary.
    pub fn root_dictionary(&self) -> Option<&plist::Dictionary> {
        self.value.as_dictionary()
    }

    /// Looks up a key without requiring it to exist, mirroring how upstream
    /// checks optional request fields before reading them.
    pub fn get(&self, key: &str) -> Option<&plist::Value> {
        self.root_dictionary()?.get(key)
    }

    /// Whether the root dictionary contains `key`.
    pub fn has_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    fn entry(&self, key: &'static str) -> Result<&plist::Value, PlistError> {
        self.get(key).ok_or(PlistError::MissingKey(key))
    }

    fn typed<'a, T>(
        &'a self,
        key: &'static str,
        expected: &'static str,
        read: impl FnOnce(&'a plist::Value) -> Option<T>,
    ) -> Result<T, PlistError> {
        read(self.entry(key)?).ok_or(PlistError::WrongType { key, expected })
    }

    /// Reads a string value.
    pub fn get_string(&self, key: &'static str) -> Result<&str, PlistError> {
        self.typed(key, "string", |value| value.as_string())
    }

    /// Reads a data value, as used for keys, IVs, proofs and authentication tags.
    pub fn get_data(&self, key: &'static str) -> Result<&[u8], PlistError> {
        self.typed(key, "data", |value| value.as_data())
    }

    /// Reads an unsigned integer value.
    pub fn get_unsigned(&self, key: &'static str) -> Result<u64, PlistError> {
        self.typed(key, "integer", |value| value.as_unsigned_integer())
    }

    /// Reads a boolean value.
    pub fn get_boolean(&self, key: &'static str) -> Result<bool, PlistError> {
        self.typed(key, "boolean", |value| value.as_boolean())
    }

    /// Reads an array value, as used for the `streams` key of a SETUP request.
    pub fn get_array(&self, key: &'static str) -> Result<&[plist::Value], PlistError> {
        self.typed(key, "array", |value| value.as_array().map(Vec::as_slice))
    }

    /// Reads a nested dictionary value.
    pub fn get_dictionary(&self, key: &'static str) -> Result<&plist::Dictionary, PlistError> {
        self.typed(key, "dictionary", |value| value.as_dictionary())
    }

    /// Serializes this value as a binary plist, the format AirPlay responses use.
    pub fn encode_binary(&self) -> Result<Vec<u8>, PlistError> {
        let mut bytes = Vec::new();
        self.value
            .to_writer_binary(&mut bytes)
            .map_err(|error| PlistError::Encode(error.to_string()))?;
        Ok(bytes)
    }

    /// Serializes this value as an XML plist.
    pub fn encode_xml(&self) -> Result<Vec<u8>, PlistError> {
        let mut bytes = Vec::new();
        self.value
            .to_writer_xml(&mut bytes)
            .map_err(|error| PlistError::Encode(error.to_string()))?;
        Ok(bytes)
    }
}

/// Builds a dictionary value from key/value pairs, preserving insertion order.
pub fn dictionary<K: Into<String>>(
    entries: impl IntoIterator<Item = (K, plist::Value)>,
) -> plist::Value {
    let mut dict = plist::Dictionary::new();
    for (key, value) in entries {
        dict.insert(key.into(), value);
    }
    plist::Value::Dictionary(dict)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> plist::Value {
        dictionary([
            ("type", plist::Value::from("101")),
            ("timingPort", plist::Value::from(7011u64)),
            ("isRemoteControlOnly", plist::Value::from(false)),
            ("ekey", plist::Value::Data(vec![0xab; 16])),
            (
                "streams",
                plist::Value::Array(vec![dictionary([
                    ("type", plist::Value::from(110u64)),
                    ("audioFormat", plist::Value::from(0x40000u64)),
                ])]),
            ),
        ])
    }

    #[test]
    fn round_trips_binary_and_xml() {
        let plist = Plist::from_value(sample());
        let binary = plist.encode_binary().unwrap();
        assert!(binary.starts_with(b"bplist"));
        assert_eq!(Plist::parse(&binary).unwrap(), plist);

        let xml = plist.encode_xml().unwrap();
        assert!(xml.starts_with(b"<?xml"));
        assert_eq!(Plist::parse(&xml).unwrap(), plist);
    }

    #[test]
    fn reads_the_typed_values_upstream_requests_use() {
        let plist = Plist::from_value(sample());
        assert_eq!(plist.get_string("type").unwrap(), "101");
        assert_eq!(plist.get_unsigned("timingPort").unwrap(), 7011);
        assert!(!plist.get_boolean("isRemoteControlOnly").unwrap());
        assert_eq!(plist.get_data("ekey").unwrap(), &[0xab; 16]);
        let streams = plist.get_array("streams").unwrap();
        assert_eq!(streams.len(), 1);
        let stream = streams[0].as_dictionary().unwrap();
        assert_eq!(stream.get("type").unwrap().as_unsigned_integer(), Some(110));
        assert!(!plist.has_key("qualifier"));
        assert!(plist.get("qualifier").is_none());
    }

    #[test]
    fn reports_missing_keys_and_type_mismatches() {
        let plist = Plist::from_value(sample());
        assert_eq!(
            plist.get_string("qualifier"),
            Err(PlistError::MissingKey("qualifier"))
        );
        assert_eq!(
            plist.get_string("timingPort"),
            Err(PlistError::WrongType {
                key: "timingPort",
                expected: "string"
            })
        );
        assert_eq!(
            plist.get_array("type"),
            Err(PlistError::WrongType {
                key: "type",
                expected: "array"
            })
        );
        assert_eq!(
            plist.get_unsigned("ekey"),
            Err(PlistError::WrongType {
                key: "ekey",
                expected: "integer"
            })
        );
    }

    #[test]
    fn rejects_oversized_and_malformed_payloads() {
        let oversized = vec![0u8; MAX_PLIST_BYTES + 1];
        assert_eq!(
            Plist::parse(&oversized),
            Err(PlistError::TooLarge {
                declared: MAX_PLIST_BYTES + 1,
                maximum: MAX_PLIST_BYTES
            })
        );
        assert!(matches!(
            Plist::parse(b"not a plist at all"),
            Err(PlistError::Decode(_))
        ));
        assert!(matches!(
            Plist::parse(b"<?xml version=\"1.0\"?><plist><dict>"),
            Err(PlistError::Decode(_))
        ));
        // A truncated binary plist must fail rather than yield partial data.
        let binary = Plist::from_value(sample()).encode_binary().unwrap();
        assert!(matches!(
            Plist::parse(&binary[..binary.len() - 4]),
            Err(PlistError::Decode(_))
        ));
    }

    #[test]
    fn root_must_be_a_dictionary_for_keyed_access() {
        let plist = Plist::parse(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><array/></plist>")
            .unwrap();
        assert!(plist.root_dictionary().is_none());
        assert_eq!(
            plist.get_string("type"),
            Err(PlistError::MissingKey("type"))
        );
    }
}
