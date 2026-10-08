//! The package website, behind a trait so tests answer with a fixture
//! registry (`Deps/Website.hs`, `Http.hs`).

use std::fmt;
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
        body(self.agent.get(url).call())
    }

    fn post(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        body(self.agent.post(url).send_empty())
    }
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
