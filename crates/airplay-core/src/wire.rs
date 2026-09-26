#![allow(missing_docs)]
// SPDX-License-Identifier: GPL-3.0-or-later
//! Incremental, bounded RTSP/HTTP wire parsing.

use std::fmt;

pub const DEFAULT_MAX_BODY: usize = 8 * 1024 * 1024;
/// Largest accepted request head: start line plus headers, including the
/// terminating CRLF CRLF. Callers accumulate the head before parsing it, so it
/// needs its own bound even though bodies already have one.
pub const DEFAULT_MAX_HEAD: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Incomplete,
    Invalid(&'static str),
    TooLarge { declared: usize, maximum: usize },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Incomplete => f.write_str("incomplete input"),
            Self::Invalid(reason) => write!(f, "invalid input: {reason}"),
            Self::TooLarge { declared, maximum } => {
                write!(f, "size {declared} exceeds maximum {maximum}")
            }
        }
    }
}
impl std::error::Error for ParseError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageHead {
    pub method: String,
    pub target: String,
    pub version: String,
    pub headers: Vec<(String, String)>,
    pub consumed: usize,
}
impl MessageHead {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
    pub fn content_length(&self, maximum: usize) -> Result<usize, ParseError> {
        let Some(value) = self.header("content-length") else {
            return Ok(0);
        };
        let length = value
            .trim()
            .parse::<usize>()
            .map_err(|_| ParseError::Invalid("content-length"))?;
        if length > maximum {
            return Err(ParseError::TooLarge {
                declared: length,
                maximum,
            });
        }
        Ok(length)
    }
}

/// Parses one request head bounded by [`DEFAULT_MAX_HEAD`].
pub fn parse_request_head(input: &[u8]) -> Result<MessageHead, ParseError> {
    parse_request_head_with_maximum(input, DEFAULT_MAX_HEAD)
}

/// Parses one request head, rejecting a header block larger than `maximum`.
///
/// Returns [`ParseError::Incomplete`] while the terminating CRLF CRLF is still
/// missing and the accumulated input is within `maximum`, and
/// [`ParseError::TooLarge`] once a head without terminator outgrows the bound or
/// the terminator itself sits past it.
pub fn parse_request_head_with_maximum(
    input: &[u8],
    maximum: usize,
) -> Result<MessageHead, ParseError> {
    let end = match input.windows(4).position(|window| window == b"\r\n\r\n") {
        Some(end) if end + 4 <= maximum => end,
        Some(end) => {
            return Err(ParseError::TooLarge {
                declared: end + 4,
                maximum,
            });
        }
        None if input.len() > maximum => {
            return Err(ParseError::TooLarge {
                declared: input.len(),
                maximum,
            });
        }
        None => return Err(ParseError::Incomplete),
    };
    let text = std::str::from_utf8(&input[..end]).map_err(|_| ParseError::Invalid("headers"))?;
    let mut lines = text.split("\r\n");
    let first = lines.next().ok_or(ParseError::Invalid("start line"))?;
    let mut fields = first.split_ascii_whitespace();
    let method = fields.next().ok_or(ParseError::Invalid("method"))?;
    let target = fields.next().ok_or(ParseError::Invalid("target"))?;
    let version = fields.next().ok_or(ParseError::Invalid("version"))?;
    if fields.next().is_some() || (!version.starts_with("RTSP/") && !version.starts_with("HTTP/")) {
        return Err(ParseError::Invalid("start line"));
    }
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(ParseError::Invalid("header"))?;
        if name.is_empty() || name.bytes().any(|byte| byte <= 0x20 || byte >= 0x7f) {
            return Err(ParseError::Invalid("header name"));
        }
        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
    }
    Ok(MessageHead {
        method: method.to_owned(),
        target: target.to_owned(),
        version: version.to_owned(),
        headers,
        consumed: end + 4,
    })
}

pub fn split_body(input: &[u8], length: usize) -> Result<&[u8], ParseError> {
    input.get(..length).ok_or(ParseError::Incomplete)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_headers_incrementally() {
        let input = b"SETUP /stream RTSP/1.0\r\nCSeq: 7\r\nContent-Length: 4\r\n\r\ndata";
        let head = parse_request_head(input).unwrap();
        assert_eq!(head.header("cseq"), Some("7"));
        assert_eq!(head.content_length(16), Ok(4));
        assert_eq!(split_body(&input[head.consumed..], 4), Ok(&b"data"[..]));
    }
    #[test]
    fn rejects_malformed_and_oversized_headers() {
        assert_eq!(
            parse_request_head(b"GET / HTTP/1.1\r\nBad\r\n\r\n"),
            Err(ParseError::Invalid("header"))
        );
        let head = parse_request_head(b"POST / HTTP/1.1\r\nContent-Length: 99\r\n\r\n").unwrap();
        assert_eq!(
            head.content_length(10),
            Err(ParseError::TooLarge {
                declared: 99,
                maximum: 10
            })
        );
    }

    #[test]
    fn reports_incomplete_and_oversized_heads() {
        assert_eq!(
            parse_request_head(b"SETUP /stream RTSP/1.0\r\nCSeq: 7\r\n"),
            Err(ParseError::Incomplete)
        );
        assert_eq!(
            parse_request_head_with_maximum(&[b'X'; 100], 16),
            Err(ParseError::TooLarge {
                declared: 100,
                maximum: 16
            })
        );
        let mut oversized = vec![b'A'; 40];
        oversized.extend_from_slice(b"\r\n\r\n");
        assert_eq!(
            parse_request_head_with_maximum(&oversized, 16),
            Err(ParseError::TooLarge {
                declared: 44,
                maximum: 16
            })
        );
    }

    #[test]
    fn rejects_bad_start_lines_and_header_encoding() {
        assert_eq!(
            parse_request_head(b"GET /\r\n\r\n"),
            Err(ParseError::Invalid("version"))
        );
        assert_eq!(
            parse_request_head(b"GET / HTTP/1.1 extra\r\n\r\n"),
            Err(ParseError::Invalid("start line"))
        );
        assert_eq!(
            parse_request_head(b"GET / FTP/1.0\r\n\r\n"),
            Err(ParseError::Invalid("start line"))
        );
        assert_eq!(
            parse_request_head(b"GET / HTTP/1.1\r\nX: \xff\r\n\r\n"),
            Err(ParseError::Invalid("headers"))
        );
        assert_eq!(
            parse_request_head(b"GET / HTTP/1.1\r\nBad Header: 1\r\n\r\n"),
            Err(ParseError::Invalid("header name"))
        );
    }

    #[test]
    fn leaves_pipelined_requests_to_the_caller() {
        let input = b"POST /x HTTP/1.1\r\nCSeq: 1\r\nContent-Length: 4\r\n\r\ndataOPTIONS * RTSP/1.0\r\nCSeq: 2\r\n\r\n";
        let first = parse_request_head(input).unwrap();
        assert_eq!(first.content_length(64), Ok(4));
        assert_eq!(split_body(&input[first.consumed..], 4).unwrap(), b"data");
        let rest = &input[first.consumed + 4..];
        let second = parse_request_head(rest).unwrap();
        assert_eq!(second.header("cseq"), Some("2"));
        assert_eq!(second.consumed, rest.len());
    }
}
