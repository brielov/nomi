//! A package website served over HTTP on the loopback interface, for
//! commands that must reach the registry, as `elm diff` must; nomi finds
//! it through `NOMI_REGISTRY`.
#![allow(dead_code)] // Each test target uses part.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

/// What answers a request: the method and path, and the body sent; `None`
/// for a 404.
pub type Handler = dyn Fn(&str, &str, &[u8]) -> Option<Vec<u8>> + Send + Sync;

/// A running server, until the test process ends.
pub struct Server {
    pub url: String,
}

impl Server {
    pub fn start(handler: Arc<Handler>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let handler = handler.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, &*handler);
                });
            }
        });
        Server { url }
    }
}

fn serve(stream: std::net::TcpStream, handler: &Handler) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let path = parts.next().unwrap_or("").to_owned();
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let mut stream = stream;
    match handler(&method, &path, &body) {
        Some(answer) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                answer.len()
            )?;
            stream.write_all(&answer)?;
        }
        None => write!(
            stream,
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )?,
    }
    stream.flush()
}
