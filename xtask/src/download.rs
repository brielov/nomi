//! Downloads that are checked against a pinned hash before anything uses them.

use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use sha1::Sha1;
use sha2::{Digest, Sha256};

/// Largest download accepted; the biggest pinned asset is about 15 MB.
const LIMIT: u64 = 256 * 1024 * 1024;

pub fn get(url: &str) -> Result<Vec<u8>> {
    eprintln!("    downloading {url}");
    let mut response = agent()
        .get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    response
        .body_mut()
        .with_config()
        .limit(LIMIT)
        .read_to_vec()
        .with_context(|| format!("reading {url}"))
}

/// Uses the system's trust store, so machines behind a TLS-inspecting proxy
/// work too; the pinned hash, not TLS, is what vouches for the bytes.
fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig};
    ureq::Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent()
}

pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub fn sha1(bytes: &[u8]) -> String {
    hex(&Sha1::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fails unless `actual` is `expected`; `what` names the download.
pub fn verify(what: &str, expected: &str, actual: &str) -> Result<()> {
    if !expected.eq_ignore_ascii_case(actual) {
        bail!("{what}: hash mismatch\n  pinned:     {expected}\n  downloaded: {actual}");
    }
    Ok(())
}

pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .context("decompressing")?;
    Ok(out)
}

/// The contents of the one regular file named `name` in a gzipped tarball.
pub fn untgz_file(bytes: &[u8], name: &str) -> Result<Vec<u8>> {
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
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
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
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha1(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn mismatch_fails() {
        let error = verify("elm", &sha256(b"a"), &sha256(b"b")).unwrap_err();
        assert!(error.to_string().contains("hash mismatch"));
        assert!(verify("elm", &sha256(b"a").to_uppercase(), &sha256(b"a")).is_ok());
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
