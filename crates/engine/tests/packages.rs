//! Package downloads on the pool, against a fixture registry.

use std::io::{Cursor, Write};
use std::sync::Mutex;
use std::time::Duration;

use nomi_base::Compat;
use nomi_engine::{Pool, packages};
use nomi_project::json::Cursor as JsonCursor;
use nomi_project::{Name, Version};
use nomi_registry::{Http, HttpError, PackageProblem, Store};
use sha1::{Digest, Sha1};

const LATEST: Compat = Compat::LATEST;

/// Publishes every elm/core release but 1.0.3 with the same archive, and
/// counts how many downloads run at once.
#[derive(Debug, Default)]
struct Website {
    archive: Vec<u8>,
    running: Mutex<(usize, usize)>,
}

impl Http for Website {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        if url.contains("/1.0.3/") {
            return Err(HttpError::Status(404));
        }
        if url.ends_with("endpoint.json") {
            let hash: String = Sha1::digest(&self.archive)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            return Ok(format!(r#"{{"url":"https://archive/","hash":"{hash}"}}"#).into_bytes());
        }
        {
            let mut running = self.running.lock().unwrap();
            running.0 += 1;
            running.1 = running.1.max(running.0);
        }
        std::thread::sleep(Duration::from_millis(50));
        self.running.lock().unwrap().0 -= 1;
        Ok(self.archive.clone())
    }

    fn post(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        panic!("an unexpected POST {url}")
    }
}

#[test]
fn releases_download_in_parallel_and_in_order() {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.add_directory("root/", options).unwrap();
    zip.start_file("root/elm.json", options).unwrap();
    zip.write_all(b"{}").unwrap();
    let website = Website {
        archive: zip.finish().unwrap().into_inner(),
        ..Website::default()
    };
    let core = Name::parse(b"elm/core", JsonCursor::start(LATEST), LATEST).unwrap();
    let releases: Vec<_> = (0..8)
        .map(|patch| (core.clone(), Version::new(1, 0, patch)))
        .collect();
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().to_path_buf());
    let pool = Pool::global();
    let results = packages::fetch(pool, &store, &website, LATEST, releases);
    assert_eq!(results.len(), 8);
    for (patch, result) in results.iter().enumerate() {
        match result {
            Err(PackageProblem::BadEndpointRequest(url, _)) => assert!(url.contains("/1.0.3/")),
            Ok(dir) => {
                assert_ne!(patch, 3);
                assert!(dir.join("elm.json").is_file());
            }
            Err(problem) => panic!("{problem:?}"),
        }
    }
    if pool.threads() > 1 {
        assert!(website.running.lock().unwrap().1 > 1);
    }
}
