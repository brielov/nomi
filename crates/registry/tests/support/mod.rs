//! A fixture registry: the package website, answered from memory.

#![allow(
    dead_code,
    reason = "shared by several test binaries, each using part of it"
)]

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use nomi_registry::{Http, HttpError};

/// Answers each `POST` with the next of its responses and each `GET` with
/// the response for its URL, and remembers every URL it was asked for.
#[derive(Debug, Default)]
pub(crate) struct Fixture {
    posts: Mutex<VecDeque<Result<Vec<u8>, HttpError>>>,
    gets: HashMap<String, Result<Vec<u8>, HttpError>>,
    asked: Mutex<Vec<String>>,
}

impl Fixture {
    pub(crate) fn new(posts: &[Result<&str, HttpError>]) -> Fixture {
        let posts = posts
            .iter()
            .map(|r| r.clone().map(|s| s.as_bytes().to_vec()))
            .collect();
        Fixture {
            posts: Mutex::new(posts),
            ..Fixture::default()
        }
    }

    /// Answers `GET url` with `response`.
    pub(crate) fn get(mut self, url: &str, response: Result<Vec<u8>, HttpError>) -> Fixture {
        self.gets.insert(url.to_owned(), response);
        self
    }

    pub(crate) fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Http for Fixture {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        self.asked.lock().unwrap().push(url.to_owned());
        self.gets
            .get(url)
            .unwrap_or_else(|| panic!("an unexpected GET {url}"))
            .clone()
    }

    fn post(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        self.asked.lock().unwrap().push(url.to_owned());
        self.posts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| panic!("an unexpected POST {url}"))
    }
}
