//! Minimal RTSP/1.0 (and HTTP/1.1) message framing for the AirPlay control
//! connection: a request line, headers, and an optional Content-Length body.

use std::io;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt};

use crate::consts;

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub uri: String,
    pub protocol: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn is_binary_plist(&self) -> bool {
        self.header("Content-Type")
            .is_some_and(|ct| ct.contains("application/x-apple-binary-plist"))
    }
}

/// Reads one request; `Ok(None)` means the peer closed the connection cleanly.
pub async fn read_request<R: AsyncBufRead + Unpin>(reader: &mut R) -> io::Result<Option<Request>> {
    let mut header_bytes = 0;
    let mut line = String::new();

    // Skip stray blank lines between messages.
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(None);
        }
        header_bytes += line.len();
        if !line.trim().is_empty() {
            break;
        }
    }

    let mut parts = line.split_whitespace();
    let (Some(method), Some(uri), Some(protocol)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(invalid(format!("bad request line: {:?}", line.trim_end())));
    };
    let (method, uri, protocol) = (method.to_string(), uri.to_string(), protocol.to_string());

    let mut headers = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        header_bytes += line.len();
        if header_bytes > MAX_HEADER_BYTES {
            return Err(invalid("request headers too large".into()));
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    let mut request = Request { method, uri, protocol, headers, body: Vec::new() };
    let length: usize = match request.header("Content-Length") {
        Some(v) => v.parse().map_err(|_| invalid(format!("bad Content-Length {v:?}")))?,
        None => 0,
    };
    if length > MAX_BODY_BYTES {
        return Err(invalid(format!("request body too large ({length} bytes)")));
    }
    request.body.resize(length, 0);
    reader.read_exact(&mut request.body).await?;
    Ok(Some(request))
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

pub struct Response {
    pub status: u16,
    pub reason: &'static str,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok() -> Self {
        Self::status(200, "OK")
    }

    pub fn status(status: u16, reason: &'static str) -> Self {
        Self { status, reason, headers: Vec::new(), body: Vec::new() }
    }

    pub fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    pub fn body(mut self, content_type: &'static str, body: Vec<u8>) -> Self {
        self.headers.push(("Content-Type", content_type.into()));
        self.body = body;
        self
    }

    pub fn plist(self, value: plist::Value) -> Self {
        let mut buf = Vec::new();
        plist::to_writer_binary(&mut buf, &value).expect("plist serialization to memory");
        self.body("application/x-apple-binary-plist", buf)
    }

    pub fn encode(&self, protocol: &str, cseq: Option<&str>) -> Vec<u8> {
        let mut head = format!("{protocol} {} {}\r\n", self.status, self.reason);
        if let Some(cseq) = cseq {
            head.push_str(&format!("CSeq: {cseq}\r\n"));
        }
        head.push_str(&format!("Server: {}\r\n", consts::SERVER_HEADER));
        for (k, v) in &self.headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str(&format!("Content-Length: {}\r\n\r\n", self.body.len()));
        let mut out = head.into_bytes();
        out.extend_from_slice(&self.body);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_request_with_body() {
        let raw = b"POST /fp-setup RTSP/1.0\r\nCSeq: 3\r\ncontent-length: 4\r\n\r\nabcdGET".to_vec();
        let mut reader = tokio::io::BufReader::new(&raw[..]);
        let req = read_request(&mut reader).await.unwrap().unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.uri, "/fp-setup");
        assert_eq!(req.header("cseq"), Some("3"));
        assert_eq!(req.body, b"abcd");
    }

    #[tokio::test]
    async fn eof_is_none() {
        let mut reader = tokio::io::BufReader::new(&b""[..]);
        assert!(read_request(&mut reader).await.unwrap().is_none());
    }
}
