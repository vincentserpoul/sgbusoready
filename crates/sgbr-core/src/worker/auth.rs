//! Pure Ed25519 helpers for device registration. Shared by the app (which
//! signs) and the future worker crate (which verifies) — no IO here.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};

use crate::error::CoreError;

/// The exact bytes signed at registration: `sgbr-register-v1:{invite_code}`.
#[must_use]
pub fn registration_message(invite_code: &str) -> Vec<u8> {
    format!("sgbr-register-v1:{invite_code}").into_bytes()
}

/// Generate a fresh Ed25519 keypair: `(secret seed, base64 pubkey)`.
///
/// # Errors
/// Returns [`CoreError::Crypto`] if the OS RNG is unavailable.
pub fn generate_keypair() -> Result<([u8; 32], String), CoreError> {
    let mut seed = [0_u8; 32];
    getrandom::fill(&mut seed).map_err(|e| CoreError::Crypto(e.to_string()))?;
    let signing_key = SigningKey::from_bytes(&seed);
    let pubkey = BASE64.encode(signing_key.verifying_key().to_bytes());
    Ok((seed, pubkey))
}

/// Sign the registration message for `invite_code` with `seed`, base64-encoded.
#[must_use]
pub fn sign_registration(seed: &[u8; 32], invite_code: &str) -> String {
    let signing_key = SigningKey::from_bytes(seed);
    let signature = signing_key.sign(&registration_message(invite_code));
    BASE64.encode(signature.to_bytes())
}

/// Verify a base64 signature over the registration message for `invite_code`,
/// given a base64-encoded pubkey. `false` on any malformed input (bad
/// base64, wrong length, bad signature) rather than an error — the caller
/// only cares "accept or reject."
#[must_use]
pub fn verify_registration(pubkey_b64: &str, invite_code: &str, signature_b64: &str) -> bool {
    let Ok(pubkey_bytes) = BASE64.decode(pubkey_b64) else {
        return false;
    };
    let Ok(pubkey_arr) = <[u8; 32]>::try_from(pubkey_bytes.as_slice()) else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&pubkey_arr) else {
        return false;
    };
    let Ok(signature_bytes) = BASE64.decode(signature_b64) else {
        return false;
    };
    let Ok(signature_arr) = <[u8; 64]>::try_from(signature_bytes.as_slice()) else {
        return false;
    };
    let signature = Signature::from_bytes(&signature_arr);
    verifying_key
        .verify(&registration_message(invite_code), &signature)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::{generate_keypair, registration_message, sign_registration, verify_registration};

    #[test]
    fn registration_message_exact_bytes() {
        assert_eq!(
            registration_message("ABCD-1234"),
            b"sgbr-register-v1:ABCD-1234".to_vec()
        );
    }

    #[test]
    fn sign_then_verify_round_trips() {
        let (seed, pubkey) = generate_keypair().expect("keygen");
        let signature = sign_registration(&seed, "ABCD-1234");
        assert!(verify_registration(&pubkey, "ABCD-1234", &signature));
    }

    #[test]
    fn tampered_signature_is_rejected() {
        let (seed, pubkey) = generate_keypair().expect("keygen");
        let mut signature = sign_registration(&seed, "ABCD-1234");
        signature.push('X');
        assert!(!verify_registration(&pubkey, "ABCD-1234", &signature));
    }

    #[test]
    fn wrong_pubkey_is_rejected() {
        let (seed, _pubkey) = generate_keypair().expect("keygen");
        let (_other_seed, other_pubkey) = generate_keypair().expect("keygen");
        let signature = sign_registration(&seed, "ABCD-1234");
        assert!(!verify_registration(&other_pubkey, "ABCD-1234", &signature));
    }

    #[test]
    fn tampered_invite_code_is_rejected() {
        let (seed, pubkey) = generate_keypair().expect("keygen");
        let signature = sign_registration(&seed, "ABCD-1234");
        assert!(!verify_registration(&pubkey, "WXYZ-5678", &signature));
    }

    #[test]
    fn malformed_pubkey_is_rejected_not_panicking() {
        let (seed, _pubkey) = generate_keypair().expect("keygen");
        let signature = sign_registration(&seed, "ABCD-1234");
        assert!(!verify_registration(
            "not-base64!!",
            "ABCD-1234",
            &signature
        ));
    }

    #[test]
    fn malformed_signature_is_rejected_not_panicking() {
        let (_seed, pubkey) = generate_keypair().expect("keygen");
        assert!(!verify_registration(&pubkey, "ABCD-1234", "not-base64!!"));
    }
}
