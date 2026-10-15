//! The package website, behind a trait so tests answer with a fixture
//! registry (`Deps/Website.hs`, `Http.hs`).

use std::fmt;
use std::io::Write as _;
use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig};

/// Where upstream's registry lives; every release hard-codes it.
pub const DOMAIN: &str = "https://package.elm-lang.org";

/// The largest response nomi reads. `/all-packages` is about a megabyte.
const LIMIT: u64 = 256 << 20;

/// How long one request may take, connecting included.
const TIMEOUT: Duration = Duration::from_secs(60);

/// A way to reach the package website and the archives it links to.
pub trait Http {
    /// The body of a successful `GET` of `url` (`Http.get`,
    /// `Http.getArchive`).
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError>;

    /// The body of a successful `POST` to `url`, sent without a body, as
    /// upstream asks for the registry (`Http.post`).
    fn post(&self, url: &str) -> Result<Vec<u8>, HttpError>;

    /// The body of a successful multipart `POST` of `parts` to `url`, as
    /// `elm publish` registers a release (`Http.upload`).
    fn upload(&self, url: &str, parts: &[Part]) -> Result<Vec<u8>, HttpError> {
        let _ = (url, parts);
        Err(HttpError::Transport(
            "this website takes no uploads".to_owned(),
        ))
    }
}

/// A part of an upload: its name, the file name it's sent as, if a file,
/// and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub file_name: Option<String>,
    pub bytes: Vec<u8>,
}

impl Part {
    /// `multipart/form-data` of `parts`, between `boundary`s.
    fn encode(parts: &[Part], boundary: &str) -> Vec<u8> {
        let mut body = Vec::new();
        for part in parts {
            let name = &part.name;
            let _ = write!(
                body,
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\""
            );
            if let Some(file) = &part.file_name {
                let _ = write!(
                    body,
                    "; filename=\"{file}\"\r\nContent-Type: application/octet-stream"
                );
            }
            body.extend_from_slice(b"\r\n\r\n");
            body.extend_from_slice(&part.bytes);
            body.extend_from_slice(b"\r\n");
        }
        let _ = write!(body, "--{boundary}--\r\n");
        body
    }
}

/// Why a request got no body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// The server answered with a status other than success.
    Status(u16),
    /// No answer: the network, TLS, a timeout or a malformed response.
    Transport(String),
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpError::Status(code) => write!(f, "the server answered with status {code}"),
            HttpError::Transport(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for HttpError {}

/// The real website, over HTTPS. Like upstream, it asks for gzip, names
/// itself in `User-Agent` and trusts the system's certificates, so
/// machines behind a TLS-inspecting proxy work; archives are vouched for
/// by their hashes, not by TLS.
#[derive(Debug)]
pub struct Web {
    agent: ureq::Agent,
}

impl Web {
    /// A client for the website, with upstream's settings.
    pub fn new() -> Web {
        let config = ureq::Agent::config_builder()
            .user_agent(concat!("nomi/", env!("CARGO_PKG_VERSION")))
            .timeout_global(Some(TIMEOUT))
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build();
        Web {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Default for Web {
    fn default() -> Web {
        Web::new()
    }
}

impl Http for Web {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        body(self.agent.get(&redirect(url)).call())
    }

    fn post(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        body(self.agent.post(&redirect(url)).send_empty())
    }

    fn upload(&self, url: &str, parts: &[Part]) -> Result<Vec<u8>, HttpError> {
        let boundary = "nomi-upload-boundary-7d3f1c";
        let content = format!("multipart/form-data; boundary={boundary}");
        body(
            self.agent
                .post(&redirect(url))
                .header("Content-Type", content)
                .send(&Part::encode(parts, boundary)[..]),
        )
    }
}

/// `url` at the website `NOMI_REGISTRY` names, when it is set, as tests
/// serve a fixture registry: [`DOMAIN`] at its root, GitHub's API under
/// `/github-api` and GitHub under `/github`.
fn redirect(url: &str) -> String {
    let Ok(registry) = std::env::var("NOMI_REGISTRY") else {
        return url.to_owned();
    };
    let registry = registry.trim_end_matches('/');
    for (host, under) in [
        (DOMAIN, ""),
        ("https://api.github.com", "/github-api"),
        ("https://github.com", "/github"),
    ] {
        if let Some(path) = url.strip_prefix(host) {
            return format!("{registry}{under}{path}");
        }
    }
    url.to_owned()
}

fn body(
    response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<Vec<u8>, HttpError> {
    response
        .map_err(error)?
        .body_mut()
        .with_config()
        .limit(LIMIT)
        .read_to_vec()
        .map_err(error)
}

fn error(e: ureq::Error) -> HttpError {
    match e {
        ureq::Error::StatusCode(code) => HttpError::Status(code),
        e => HttpError::Transport(e.to_string()),
    }
}
