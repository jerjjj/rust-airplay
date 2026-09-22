// SPDX-License-Identifier: GPL-3.0-or-later
//! Pure RTSP control-session state machine.
//!
//! The state machine owns one AirPlay control connection. It consumes one
//! complete request at a time and produces a bounded response; socket I/O and
//! service discovery remain outside this module.

#![allow(missing_docs)]

use std::fmt;

use crate::wire::{MessageHead, ParseError, parse_request_head, split_body};

const MAX_CONTROL_BODY: usize = 64 * 1024;
const SESSION_ID: &str = "rust-airplay";

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
    InvalidState { method: String, state: ControlState },
    SessionMismatch,
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => error.fmt(f),
            Self::MissingHeader(name) => write!(f, "missing RTSP header {name}"),
            Self::InvalidHeader(name) => write!(f, "invalid RTSP header {name}"),
            Self::InvalidState { method, state } => {
                write!(f, "RTSP method {method} is invalid in state {state:?}")
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

    /// Handle one complete RTSP request and advance this connection's state.
    pub fn handle(&mut self, input: &[u8]) -> Result<ControlResponse, ControlError> {
        let head = parse_request_head(input)?;
        let length = head.content_length(MAX_CONTROL_BODY)?;
        let body = split_body(&input[head.consumed..], length)?;
        let cseq = required_u32(&head, "cseq")?;
        if matches!(self.state, ControlState::Setup | ControlState::Recording)
            && head.method != "OPTIONS"
            && head.method != "SETUP"
        {
            let session = head
                .header("session")
                .ok_or(ControlError::MissingHeader("Session"))?;
            if session != SESSION_ID {
                return Err(ControlError::SessionMismatch);
            }
        }
        self.validate_body(&head, body)?;
        self.advance(&head.method)?;

        let mut response = ControlResponse {
            status: 200,
            reason: "OK",
            headers: vec![("CSeq".to_owned(), cseq.to_string())],
            body: Vec::new(),
        };
        if self.state == ControlState::Setup || self.state == ControlState::Recording {
            response
                .headers
                .push(("Session".to_owned(), SESSION_ID.to_owned()));
        }
        if head.method == "OPTIONS" {
            response.headers.push((
                "Public".to_owned(),
                "OPTIONS, ANNOUNCE, SETUP, RECORD, FLUSH, TEARDOWN".to_owned(),
            ));
        }
        Ok(response)
    }

    fn validate_body(&self, head: &MessageHead, body: &[u8]) -> Result<(), ControlError> {
        if matches!(head.method.as_str(), "ANNOUNCE" | "SETUP") && body.is_empty() {
            return Err(ControlError::InvalidHeader("Content-Length"));
        }
        Ok(())
    }

    fn advance(&mut self, method: &str) -> Result<(), ControlError> {
        let next = match (self.state, method) {
            (ControlState::New, "OPTIONS") => ControlState::New,
            (ControlState::New, "ANNOUNCE") => ControlState::Announced,
            (ControlState::Announced, "SETUP") => ControlState::Setup,
            (ControlState::Setup, "RECORD") => ControlState::Recording,
            (ControlState::Recording, "FLUSH") => ControlState::Recording,
            (ControlState::Setup | ControlState::Recording, "TEARDOWN") => ControlState::Closed,
            (state, _) => {
                return Err(ControlError::InvalidState {
                    method: method.to_owned(),
                    state,
                });
            }
        };
        self.state = next;
        Ok(())
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

    fn request(method: &str, cseq: u32, extra: &str, body: &[u8]) -> Vec<u8> {
        format!(
            "{method} /stream RTSP/1.0\r\nCSeq: {cseq}\r\n{extra}Content-Length: {}\r\n\r\n",
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
        let options = session.handle(&request("OPTIONS", 1, "", &[])).unwrap();
        assert_eq!(session.state(), ControlState::New);
        assert!(
            String::from_utf8(options.to_bytes())
                .unwrap()
                .contains("Public:")
        );
        session
            .handle(&request("ANNOUNCE", 2, "", b"v=0\r\n"))
            .unwrap();
        session
            .handle(&request("SETUP", 3, "", b"transport"))
            .unwrap();
        let response = session
            .handle(&request("RECORD", 4, "Session: rust-airplay\r\n", &[]))
            .unwrap();
        assert_eq!(session.state(), ControlState::Recording);
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name == "Session" && value == SESSION_ID)
        );
        session
            .handle(&request("FLUSH", 5, "Session: rust-airplay\r\n", &[]))
            .unwrap();
        session
            .handle(&request("TEARDOWN", 6, "Session: rust-airplay\r\n", &[]))
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
        session.handle(&request("ANNOUNCE", 1, "", b"x")).unwrap();
        session.handle(&request("SETUP", 2, "", b"x")).unwrap();
        assert!(matches!(
            session.handle(&request("RECORD", 3, "Session: wrong\r\n", &[])),
            Err(ControlError::SessionMismatch)
        ));
    }
}
