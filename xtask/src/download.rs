//! Downloads that are checked against a pinned hash before anything uses them.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// Largest download accepted; the biggest pinned asset is about 15 MB.
const LIMIT: u64 = 256 * 1024 * 1024;

/// The body of a `GET` of `url`, saying so on standard error.
pub(crate) fn get(url: &str) -> Result<Vec<u8>> {
    eprintln!("    downloading {url}");
    get_quiet(url)
}

/// [`get`] without the progress line, for bulk downloads.
pub(crate) fn get_quiet(url: &str) -> Result<Vec<u8>> {
    let response = agent().get(url).call();
    body(response.with_context(|| format!("GET {url}"))?, url)
}

/// The body of an empty `POST` to `url`, as the package website answers
/// `/all-packages`.
pub(crate) fn post_quiet(url: &str) -> Result<Vec<u8>> {
    let response = agent().post(url).send_empty();
    body(response.with_context(|| format!("POST {url}"))?, url)
}

fn body(mut response: ureq::http::Response<ureq::Body>, url: &str) -> Result<Vec<u8>> {
    (response.body_mut().with_config().limit(LIMIT).read_to_vec())
        .with_context(|| format!("reading {url}"))
}

/// Uses the system's trust store, so machines behind a TLS-inspecting proxy
/// work too; the pinned hash, not TLS, is what vouches for the bytes.
fn agent() -> &'static ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig};
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .new_agent()
    })
}

/// GitHub's zipball of `name` at the tag `version`, which the package
/// website's `endpoint.json` hashes.
pub(crate) fn zipball(name: &str, version: &str) -> String {
    format!("https://github.com/{name}/zipball/{version}/")
}

/// Whether the stamp file `stamp` holds exactly `expected`.
pub(crate) fn stamped(stamp: &Path, expected: &str) -> bool {
    fs::read_to_string(stamp).ok().as_deref() == Some(expected)
}

/// A download whose hash isn't the pinned one.
#[derive(Debug)]
pub(crate) struct HashMismatch {
    what: String,
    pinned: String,
    downloaded: String,
}

impl fmt::Display for HashMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let HashMismatch {
            what,
            pinned,
            downloaded,
        } = self;
        write!(
            f,
            "{what}: hash mismatch\n  pinned:     {pinned}\n  downloaded: {downloaded}"
        )
    }
}

impl std::error::Error for HashMismatch {}

/// Fails with a [`HashMismatch`] unless `actual` is `expected`; `what`
/// names the download.
pub(crate) fn verify(what: &str, expected: &str, actual: &str) -> Result<()> {
    if !expected.eq_ignore_ascii_case(actual) {
        return Err(HashMismatch {
            what: what.to_owned(),
            pinned: expected.to_owned(),
            downloaded: actual.to_owned(),
        }
        .into());
    }
    Ok(())
}

/// The bytes a gzip stream holds.
pub(crate) fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .context("decompressing")?;
    Ok(out)
}

/// The contents of the one regular file named `name` in a gzipped tarball.
pub(crate) fn untgz_file(bytes: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    for entry in archive.entries().context("reading tarball")? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if entry.header().entry_type().is_file() && path.file_name() == Some(name.as_ref()) {
            let mut out = Vec::new();
            entry.read_to_end(&mut out)?;
            return Ok(out);
        }
    }
    bail!("tarball has no file named {name}")
}

/// Writes `bytes` next to `path` and renames it into place, so a failed run
/// never leaves a half-written file behind.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("path has no parent")?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = path.with_extension("partial");
    fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        assert_eq!(
            nomi_testkit::sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            nomi_registry::store::sha1(b"abc"),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn mismatch_fails() {
        let error = verify(
            "elm",
            &nomi_testkit::sha256_hex(b"a"),
            &nomi_testkit::sha256_hex(b"b"),
        )
        .unwrap_err();
        assert!(error.downcast_ref::<HashMismatch>().is_some());
        assert!(error.to_string().contains("hash mismatch"));
        assert!(
            verify(
                "elm",
                &nomi_testkit::sha256_hex(b"a").to_uppercase(),
                &nomi_testkit::sha256_hex(b"a")
            )
            .is_ok()
        );
    }

    #[test]
    fn tarball_member_is_found() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "dir/tool", &b"bin"[..])
            .unwrap();
        let tar = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar).unwrap();
        let tgz = gz.finish().unwrap();
        assert_eq!(untgz_file(&tgz, "tool").unwrap(), b"bin");
        assert!(untgz_file(&tgz, "other").is_err());
    }
}
