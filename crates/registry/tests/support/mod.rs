//! A fixture registry: the package website, answered from memory.

use std::collections::HashMap;
use std::sync::Mutex;

use nomi_registry::{Http, HttpError};

/// Answers each request with the response for its URL, whether `GET` or
/// `POST`, and remembers every URL it was asked for.
#[derive(Debug, Default)]
pub(crate) struct Fixture {
    responses: HashMap<String, Result<Vec<u8>, HttpError>>,
    asked: Mutex<Vec<String>>,
}

impl Fixture {
    /// Answers `url` with `body`.
    pub(crate) fn answering(mut self, url: &str, body: impl AsRef<[u8]>) -> Fixture {
        let body = body.as_ref().to_vec();
        self.responses.insert(url.to_owned(), Ok(body));
        self
    }

    /// Fails requests for `url` with `error`.
    pub(crate) fn failing(mut self, url: &str, error: HttpError) -> Fixture {
        self.responses.insert(url.to_owned(), Err(error));
        self
    }

    /// The URLs asked for, in order.
    pub(crate) fn asked(&self) -> Vec<String> {
        nomi_base::lock(&self.asked).clone()
    }

    fn answer(&self, method: &str, url: &str) -> Result<Vec<u8>, HttpError> {
        nomi_base::lock(&self.asked).push(url.to_owned());
        (self.responses.get(url).cloned()).unwrap_or_else(|| panic!("an unexpected {method} {url}"))
    }
}

impl Http for Fixture {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        self.answer("GET", url)
    }

    fn post(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        self.answer("POST", url)
    }
}
