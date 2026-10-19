//! Offline license keys for Orbo Pro: a JSON payload signed with Ed25519.
//! The app only holds the public key, so keys can be checked without any server.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Public half of the signing key. The private half never enters the repository.
pub const PUBLIC_KEY: &str = include_str!("public_key.txt");

const PREFIX: &str = "ORBO-";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct License {
    /// Format version of the payload.
    pub v: u8,
    pub id: String,
    pub name: String,
    pub email: String,
    /// "pro" for now; room for "team" later.
    pub plan: String,
    /// ISO date (YYYY-MM-DD).
    pub issued: String,
    /// Versions released up to this ISO date are covered.
    pub updates_until: String,
}

#[derive(Debug, PartialEq)]
pub enum LicenseError {
    Format,
    Signature,
}

impl std::fmt::Display for LicenseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LicenseError::Format => write!(f, "Das ist kein gültiger Orbo-Lizenzschlüssel."),
            LicenseError::Signature => write!(f, "Der Lizenzschlüssel ist beschädigt oder nicht echt."),
        }
    }
}

impl std::error::Error for LicenseError {}

fn verifying_key(public_b64: &str) -> Result<VerifyingKey, LicenseError> {
    let bytes: [u8; 32] = URL_SAFE_NO_PAD.decode(public_b64.trim()).map_err(|_| LicenseError::Format)?.try_into().map_err(|_| LicenseError::Format)?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| LicenseError::Format)
}

/// Accepts keys with line breaks, spaces or surrounding text from e-mails.
fn normalize(key: &str) -> String {
    let compact: String = key.chars().filter(|c| !c.is_whitespace()).collect();
    let Some(start) = compact.find(PREFIX) else { return compact };
    let rest = &compact[start..];
    // An Ed25519 signature is 64 bytes, i.e. exactly 86 base64url characters; text after it is noise.
    match rest.find('.') {
        Some(dot) => rest.chars().take(dot + 1 + 86).collect(),
        None => rest.to_string(),
    }
}

pub fn verify_with(key: &str, public_b64: &str) -> Result<License, LicenseError> {
    let key = normalize(key);
    let body = key.strip_prefix(PREFIX).ok_or(LicenseError::Format)?;
    let (payload_b64, sig_b64) = body.split_once('.').ok_or(LicenseError::Format)?;
    let payload = URL_SAFE_NO_PAD.decode(payload_b64).map_err(|_| LicenseError::Format)?;
    let sig_bytes: [u8; 64] = URL_SAFE_NO_PAD.decode(sig_b64).map_err(|_| LicenseError::Format)?.try_into().map_err(|_| LicenseError::Format)?;
    verifying_key(public_b64)?.verify(&payload, &Signature::from_bytes(&sig_bytes)).map_err(|_| LicenseError::Signature)?;
    serde_json::from_slice(&payload).map_err(|_| LicenseError::Format)
}

pub fn verify(key: &str) -> Result<License, LicenseError> {
    verify_with(key, PUBLIC_KEY)
}

pub fn issue(license: &License, signing_key: &SigningKey) -> String {
    let payload = serde_json::to_vec(license).expect("license serializes");
    let sig = signing_key.sign(&payload);
    format!("{PREFIX}{}.{}", URL_SAFE_NO_PAD.encode(payload), URL_SAFE_NO_PAD.encode(sig.to_bytes()))
}

pub fn generate_signing_key() -> SigningKey {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).expect("system randomness");
    SigningKey::from_bytes(&seed)
}

pub fn encode_signing_key(k: &SigningKey) -> String {
    URL_SAFE_NO_PAD.encode(k.to_bytes())
}

pub fn decode_signing_key(s: &str) -> anyhow::Result<SigningKey> {
    let bytes: [u8; 32] = URL_SAFE_NO_PAD.decode(s.trim())?.try_into().map_err(|_| anyhow::anyhow!("Schlüssel hat nicht 32 Byte"))?;
    Ok(SigningKey::from_bytes(&bytes))
}

pub fn encode_public_key(k: &SigningKey) -> String {
    URL_SAFE_NO_PAD.encode(k.verifying_key().to_bytes())
}

/// Pro is active for a build released on or before `updates_until`.
pub fn covers_release(license: &License, release_date: &str) -> bool {
    release_date <= license.updates_until.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> License {
        License {
            v: 1,
            id: "T-1".into(),
            name: "Test Kunde".into(),
            email: "kunde@example.com".into(),
            plan: "pro".into(),
            issued: "2026-10-18".into(),
            updates_until: "2027-10-18".into(),
        }
    }

    #[test]
    fn roundtrip_and_tamper() {
        let sk = generate_signing_key();
        let pk = encode_public_key(&sk);
        let key = issue(&sample(), &sk);
        assert_eq!(verify_with(&key, &pk).unwrap(), sample());

        // Pasted from an e-mail with line breaks and text around it.
        let messy = format!("Dein Schlüssel:\n{}\n{}\nViel Spaß", &key[..40], &key[40..]);
        assert_eq!(verify_with(&messy, &pk).unwrap(), sample());

        let mut tampered = sample();
        tampered.updates_until = "2099-01-01".into();
        let forged = format!("{PREFIX}{}.{}", URL_SAFE_NO_PAD.encode(serde_json::to_vec(&tampered).unwrap()), key.split_once('.').unwrap().1);
        assert_eq!(verify_with(&forged, &pk), Err(LicenseError::Signature));

        let other = encode_public_key(&generate_signing_key());
        assert_eq!(verify_with(&key, &other), Err(LicenseError::Signature));
        assert_eq!(verify_with("hallo", &pk), Err(LicenseError::Format));
    }

    #[test]
    fn update_window() {
        let l = sample();
        assert!(covers_release(&l, "2027-10-18"));
        assert!(!covers_release(&l, "2027-10-19"));
    }
}
