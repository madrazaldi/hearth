//! Client certificates for SASL EXTERNAL / CertFP.

use anyhow::{anyhow, Context, Result};
use rustls::pki_types::CertificateDer;
use serde::Serialize;
use sha2::{Digest, Sha256, Sha512};
use std::path::Path;

#[derive(Debug, Serialize)]
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
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["hearth".to_string()]).context("generating certificate")?;
    let pem = format!("{}{}", cert.pem(), signing_key.serialize_pem());
    let path = dir.join(format!("{safe}.pem"));
    std::fs::write(&path, pem)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_a_loadable_identity() {
        let dir = std::env::temp_dir().join(format!("hearth-cert-{}", uuid::Uuid::new_v4().simple()));
        let info = generate(&dir, "srv1").unwrap();
        assert_eq!(info.sha256.len(), 64);
        assert_eq!(info.sha512.len(), 128);
        assert!(crate::irc::transport::load_identity(&info.path).is_ok());
        assert_eq!(super::info(&info.path).unwrap().sha512, info.sha512);
        std::fs::remove_dir_all(dir).ok();
    }
}
