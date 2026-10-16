//! SASL SCRAM-SHA-256 (RFC 5802 / RFC 7677), client side, without channel
//! binding.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub struct Scram {
    password: String,
    client_first_bare: String,
    cnonce: String,
    /// Expected server signature, known after the client-final message.
    server_signature: Option<Vec<u8>>,
}

impl Scram {
    pub fn new(user: &str, password: &str) -> Self {
        let mut bytes = [0u8; 18];
        if getrandom::fill(&mut bytes).is_err() {
            // getrandom only fails on exotic platforms; fall back to a v4 UUID.
            bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        }
        Self::with_nonce(user, password, &B64.encode(bytes))
    }

    pub fn with_nonce(user: &str, password: &str, cnonce: &str) -> Self {
        let name = user.replace('=', "=3D").replace(',', "=2C");
        Scram {
            password: password.to_string(),
            client_first_bare: format!("n={name},r={cnonce}"),
            cnonce: cnonce.to_string(),
            server_signature: None,
        }
    }

    pub fn client_first(&self) -> String {
        format!("n,,{}", self.client_first_bare)
    }

    /// Answer the server-first message with the client-final message.
    pub fn client_final(&mut self, server_first: &str) -> Result<String, String> {
        let attr = |k: &str| {
            server_first
                .split(',')
                .find_map(|kv| kv.strip_prefix(k).and_then(|v| v.strip_prefix('=')))
                .ok_or_else(|| format!("server-first message is missing '{k}'"))
        };
        let nonce = attr("r")?;
        if !nonce.starts_with(&self.cnonce) || nonce.len() <= self.cnonce.len() {
            return Err("server nonce does not extend the client nonce".into());
        }
        let salt = B64.decode(attr("s")?).map_err(|_| "invalid salt".to_string())?;
        let iterations: u32 = attr("i")?.parse().map_err(|_| "invalid iteration count".to_string())?;
        if iterations == 0 || iterations > 1_000_000 {
            return Err("unreasonable iteration count".into());
        }

        let mut salted = [0u8; 32];
        pbkdf2::pbkdf2::<HmacSha256>(self.password.as_bytes(), &salt, iterations, &mut salted)
            .map_err(|_| "key derivation failed".to_string())?;
        let client_key = hmac(&salted, b"Client Key");
        let stored_key = Sha256::digest(&client_key);
        let without_proof = format!("c=biws,r={nonce}");
        let auth_message = format!("{},{server_first},{without_proof}", self.client_first_bare);
        let signature = hmac(&stored_key, auth_message.as_bytes());
        let proof: Vec<u8> = client_key.iter().zip(&signature).map(|(a, b)| a ^ b).collect();
        let server_key = hmac(&salted, b"Server Key");
        self.server_signature = Some(hmac(&server_key, auth_message.as_bytes()));
        Ok(format!("{without_proof},p={}", B64.encode(proof)))
    }

    /// Check the server-final message proves the server knows the password.
    pub fn verify(&self, server_final: &str) -> Result<(), String> {
        if let Some(e) = server_final.strip_prefix("e=") {
            return Err(format!("server rejected authentication: {e}"));
        }
        let v = server_final.strip_prefix("v=").ok_or("malformed server-final message")?;
        let got = B64.decode(v.split(',').next().unwrap_or("")).map_err(|_| "malformed server signature")?;
        match &self.server_signature {
            Some(want) if *want == got => Ok(()),
            _ => Err("server signature mismatch".into()),
        }
    }
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7677_vector() {
        let mut s = Scram::with_nonce("user", "pencil", "rOprNGfwEbeRWgbNEkqO");
        assert_eq!(s.client_first(), "n,,n=user,r=rOprNGfwEbeRWgbNEkqO");
        let server_first = "r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096";
        assert_eq!(
            s.client_final(server_first).unwrap(),
            "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
        );
        assert!(s.verify("v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=").is_ok());
        assert!(s.verify("v=AAAA").is_err());
        assert!(s.verify("e=invalid-proof").is_err());
    }

    #[test]
    fn rejects_foreign_nonce_and_escapes_names() {
        let mut s = Scram::with_nonce("a,b=c", "pw", "abc");
        assert_eq!(s.client_first(), "n,,n=a=2Cb=3Dc,r=abc");
        assert!(s.client_final("r=zzz,s=AAAA,i=4096").is_err());
        assert!(s.client_final("r=abcdef,s=AAAA,i=0").is_err());
    }
}
