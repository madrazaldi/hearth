//! Client certificates for SASL EXTERNAL / CertFP.

use anyhow::{anyhow, Context, Result};
use rustls::pki_types::CertificateDer;
use serde::Serialize;
use sha2::{Digest, Sha256, Sha512};
use std::path::Path;
#[cfg(test)]
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertInfo {
    pub path: String,
    pub sha256: String,
    pub sha512: String,
}

/// Create a self-signed certificate and key in `dir/<name>.pem` (mode 0600).
pub fn generate(dir: &Path, name: &str) -> Result<CertInfo> {
    let safe: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    if safe.is_empty() {
        return Err(anyhow!("invalid certificate name"));
    }
    std::fs::create_dir_all(dir)?;
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(vec!["hearth".to_string()]).context("generating certificate")?;
    let pem = format!("{}{}", cert.pem(), signing_key.serialize_pem());
    let path = dir.join(format!("{safe}.pem"));
    crate::config::write_private(&path, pem.as_bytes())?;
    info(&path.to_string_lossy())
}

/// Fingerprints of the first certificate in a PEM file, as NickServ shows them.
pub fn info(path: &str) -> Result<CertInfo> {
    let (certs, _) = crate::irc::transport::load_identity(path)?;
    Ok(fingerprints(path, &certs[0]))
}

fn fingerprints(path: &str, der: &CertificateDer) -> CertInfo {
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    CertInfo { path: path.to_string(), sha256: hex(&Sha256::digest(der)), sha512: hex(&Sha512::digest(der)) }
}

/// The fingerprints of a certificate seen elsewhere, such as the one a peer presented.
#[cfg(test)]
pub(crate) fn fingerprints_of(der: &CertificateDer) -> CertInfo {
    fingerprints("", der)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::irc::transport::load_identity;
    use crate::test_util::TempDir;

    #[test]
    fn generates_a_loadable_identity() {
        let dir = TempDir::new();
        let made = generate(dir.path(), "srv1").unwrap();
        assert_eq!(PathBuf::from(&made.path), dir.path().join("srv1.pem"));
        assert_eq!(made.sha256.len(), 64);
        assert_eq!(made.sha512.len(), 128);
        assert!(load_identity(&made.path).is_ok());
        assert_eq!(info(&made.path).unwrap().sha512, made.sha512);
    }

    #[test]
    fn writes_the_certificate_then_the_key_readable_only_by_the_owner() {
        let dir = TempDir::new();
        let made = generate(dir.path(), "srv").unwrap();
        let text = std::fs::read_to_string(&made.path).unwrap();
        assert!(text.find("BEGIN CERTIFICATE").unwrap() < text.find("BEGIN PRIVATE KEY").unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&made.path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn the_key_matches_the_certificate_and_tls_accepts_the_pair() {
        let dir = TempDir::new();
        let made = generate(dir.path(), "srv").unwrap();
        let (certs, key) = load_identity(&made.path).unwrap();
        assert_eq!(certs.len(), 1);
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_client_auth_cert(certs, key);
        assert!(config.is_ok(), "{:?}", config.err());
    }

    #[test]
    fn fingerprints_are_the_digests_of_the_der_certificate() {
        let dir = TempDir::new();
        let made = generate(dir.path(), "srv").unwrap();
        let (certs, _) = load_identity(&made.path).unwrap();
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        assert_eq!(made.sha256, hex(&Sha256::digest(&certs[0])));
        assert_eq!(made.sha512, hex(&Sha512::digest(&certs[0])));
    }

    #[test]
    fn every_certificate_is_different() {
        let dir = TempDir::new();
        assert_ne!(generate(dir.path(), "a").unwrap().sha256, generate(dir.path(), "b").unwrap().sha256);
    }

    #[test]
    fn names_are_sanitised_and_an_empty_result_is_refused() {
        let dir = TempDir::new();
        assert_eq!(PathBuf::from(generate(dir.path(), "../../etc/pa ss").unwrap().path), dir.path().join("etcpass.pem"));
        for bad in ["../..", ""] {
            assert!(generate(dir.path(), bad).unwrap_err().to_string().contains("invalid certificate name"), "{bad:?}");
        }
    }

    fn identity_file(dir: &TempDir, name: &str, text: &str) -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn pems(dir: &TempDir) -> (String, String) {
        let made = generate(dir.path(), "src").unwrap();
        let text = std::fs::read_to_string(made.path).unwrap();
        let at = text.find("-----BEGIN PRIVATE KEY").unwrap();
        (text[..at].to_string(), text[at..].to_string())
    }

    #[test]
    fn a_certificate_without_a_key_is_refused() {
        let dir = TempDir::new();
        let (cert, _) = pems(&dir);
        let path = identity_file(&dir, "nokey.pem", &cert);
        let err = load_identity(&path).err().unwrap();
        assert!(format!("{err:#}").contains("no usable private key"), "{err:#}");
    }

    #[test]
    fn accepts_a_chain_and_a_key_before_the_certificates() {
        let dir = TempDir::new();
        let (a_cert, a_key) = pems(&dir);
        let (b_cert, _) = pems(&dir);
        let path = identity_file(&dir, "chain.pem", &format!("{a_key}{a_cert}{b_cert}"));
        let (certs, _) = load_identity(&path).unwrap();
        assert_eq!(certs.len(), 2);
        assert_ne!(certs[0], certs[1]);
    }

    #[test]
    fn garbled_certificate_data_is_reported() {
        let dir = TempDir::new();
        let path = identity_file(&dir, "bad.pem", "-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----\n");
        let err = load_identity(&path).err().unwrap();
        assert!(format!("{err:#}").contains("invalid client certificate"), "{err:#}");
    }
}
