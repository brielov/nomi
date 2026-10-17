//! HTTP/1.1 to a server on this machine over a plain socket, so a test
//! sends exactly the request line it means, `..` and all; and an event
//! stream's lines as they come.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use crate::process::lines;

/// A server's answer.
#[derive(Debug)]
pub struct Response {
    pub status: u16,
    /// Each header's name, in lower case, and value.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    /// The value of the header `name`, in lower case, if given.
    pub fn header(&self, name: &str) -> Option<&str> {
        (self.headers.iter())
            .find(|(n, _)| n == name)
            .map(|(_, value)| value.as_str())
    }

    /// The body as text.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Sends `method` of `target` to the server on `port` of 127.0.0.1 and
/// reads its whole answer, the connection closed after it.
pub fn request(port: u16, method: &str, target: &str) -> io::Result<Response> {
    request_to(&format!("127.0.0.1:{port}"), port, method, target)
}

/// [`request`], its `Host` header `host`.
pub fn request_to(host: &str, port: u16, method: &str, target: &str) -> io::Result<Response> {
    let stream = connect(host, port, method, target, "close")?;
    read(stream)
}

/// A request that closes its connection, with `headers`, each ending in
/// a line break, and `body`.
pub fn closing(method: &str, target: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut request = format!(
        "{method} {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{headers}Content-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    request.extend_from_slice(body);
    request
}

/// The answer `stream` holds up to its end, as a server closing the
/// connection after it sends.
pub fn read(stream: impl Read) -> io::Result<Response> {
    let mut reader = BufReader::new(stream);
    let (status, headers) = head(&mut reader)?;
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest)?;
    let chunked = (headers.iter()).any(|(n, v)| n == "transfer-encoding" && v == "chunked");
    // An answer to HEAD says how it would send a body it doesn't.
    let body = if chunked && !rest.is_empty() {
        unchunked(&rest)?
    } else {
        rest
    };
    Ok(Response {
        status,
        headers,
        body,
    })
}

/// Opens the event stream at `target` on `port`: its status, and each
/// line after its head as it comes.
pub fn events(port: u16, target: &str) -> io::Result<(u16, Receiver<String>)> {
    let stream = connect(
        &format!("127.0.0.1:{port}"),
        port,
        "GET",
        target,
        "keep-alive",
    )?;
    stream.set_read_timeout(None)?;
    let mut reader = BufReader::new(stream);
    let (status, _) = head(&mut reader)?;
    Ok((status, lines(reader)))
}

fn connect(
    host: &str,
    port: u16,
    method: &str,
    target: &str,
    connection: &str,
) -> io::Result<TcpStream> {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    write!(
        stream,
        "{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: {connection}\r\n\r\n"
    )?;
    Ok(stream)
}

/// The status and headers at the start of a response.
fn head(reader: &mut impl BufRead) -> io::Result<(u16, Vec<(String, String)>)> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_owned());
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status = (line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| bad(&format!("no status in {line:?}")))?;
    let mut headers = Vec::new();
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            return Ok((status, headers));
        }
        let (name, value) = (line.split_once(':')).ok_or_else(|| bad(line))?;
        headers.push((name.to_ascii_lowercase(), value.trim().to_owned()));
    }
}

/// The body of a chunked transfer.
fn unchunked(mut rest: &[u8]) -> io::Result<Vec<u8>> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "a bad chunk");
    let mut body = Vec::new();
    loop {
        let end = rest.windows(2).position(|w| w == b"\r\n").ok_or_else(bad)?;
        let size = std::str::from_utf8(&rest[..end]).map_err(|_| bad())?;
        let size = usize::from_str_radix(size.split(';').next().unwrap_or_default().trim(), 16)
            .map_err(|_| bad())?;
        rest = &rest[end + 2..];
        if size == 0 {
            return Ok(body);
        }
        body.extend_from_slice(rest.get(..size).ok_or_else(bad)?);
        rest = rest.get(size + 2..).ok_or_else(bad)?;
    }
}
