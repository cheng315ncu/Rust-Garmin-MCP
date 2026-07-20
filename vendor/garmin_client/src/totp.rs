//! TOTP code generation (RFC 6238), so MFA can be satisfied without a terminal.
//!
//! Upstream reads the MFA code from stdin. That cannot work under MCP, where
//! stdin carries the JSON-RPC transport — the prompt would consume protocol
//! bytes. Generating the code from the account's TOTP secret removes the
//! interactive step entirely, so MFA stays enabled on the Garmin account.
//!
//! This is the same construction an authenticator app runs: HMAC-SHA1 over the
//! 30-second counter, dynamically truncated to six digits. The implementation is
//! checked against RFC 6238's published test vectors in the tests below.

use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

pub const DEFAULT_PERIOD: u64 = 30;
pub const DEFAULT_DIGITS: u32 = 6;

#[derive(Debug)]
pub enum TotpError {
    /// The secret was not valid base32.
    BadSecret,
}

impl std::fmt::Display for TotpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TotpError::BadSecret => write!(
                f,
                "GARMIN_TOTP_SECRET is not valid base32. Use the secret shown when \
                 you set up two-factor authentication (the string behind the QR \
                 code), not a six-digit code."
            ),
        }
    }
}

impl std::error::Error for TotpError {}

/// Decode a user-supplied base32 secret.
///
/// Authenticator secrets are shown in many shapes — lowercase, space- or
/// hyphen-grouped, with or without `=` padding — so normalise before decoding
/// rather than making the user clean it up.
pub fn decode_secret(secret: &str) -> Result<Vec<u8>, TotpError> {
    let cleaned: String = secret
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '=')
        .collect::<String>()
        .to_uppercase();

    if cleaned.is_empty() {
        return Err(TotpError::BadSecret);
    }

    base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &cleaned)
        .filter(|bytes| !bytes.is_empty())
        .ok_or(TotpError::BadSecret)
}

/// The HOTP value for an explicit counter (RFC 4226).
pub fn hotp(key: &[u8], counter: u64, digits: u32) -> u32 {
    let mut mac = HmacSha1::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();

    // Dynamic truncation: the low nibble of the last byte picks the offset.
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = ((u32::from(digest[offset]) & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);

    binary % 10u32.pow(digits)
}

/// The TOTP code for a given Unix timestamp.
pub fn totp_at(key: &[u8], unix_seconds: u64, period: u64, digits: u32) -> String {
    let counter = unix_seconds / period.max(1);
    format!(
        "{:0width$}",
        hotp(key, counter, digits),
        width = digits as usize
    )
}

/// The TOTP code for right now, from a base32 secret.
pub fn current_code(secret: &str) -> Result<String, TotpError> {
    let key = decode_secret(secret)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(totp_at(&key, now, DEFAULT_PERIOD, DEFAULT_DIGITS))
}

/// Seconds until the current code rolls over. Used to avoid submitting a code
/// that expires mid-request.
pub fn seconds_remaining() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    DEFAULT_PERIOD - (now % DEFAULT_PERIOD)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 Appendix B uses this ASCII seed for the SHA-1 vectors.
    const RFC_SEED: &[u8] = b"12345678901234567890";

    #[test]
    fn matches_rfc6238_sha1_test_vectors() {
        // (unix time, expected 8-digit TOTP) straight from RFC 6238 Appendix B.
        let vectors: &[(u64, &str)] = &[
            (59, "94287082"),
            (1_111_111_109, "07081804"),
            (1_111_111_111, "14050471"),
            (1_234_567_890, "89005924"),
            (2_000_000_000, "69279037"),
            (20_000_000_000, "65353130"),
        ];
        for (time, expected) in vectors {
            assert_eq!(
                totp_at(RFC_SEED, *time, 30, 8),
                *expected,
                "RFC 6238 vector at t={time}"
            );
        }
    }

    #[test]
    fn six_digit_codes_are_the_low_six_of_the_rfc_vectors() {
        for (time, expected8) in [(59u64, "94287082"), (1_234_567_890, "89005924")] {
            let six = totp_at(RFC_SEED, time, 30, 6);
            assert_eq!(six, expected8[2..], "t={time}");
            assert_eq!(six.len(), 6);
        }
    }

    #[test]
    fn codes_are_zero_padded_to_full_width() {
        // A truncated value below 100000 must still render six characters.
        let key = decode_secret("JBSWY3DPEHPK3PXP").unwrap();
        for t in (0..300_000u64).step_by(30) {
            let code = totp_at(&key, t, 30, 6);
            assert_eq!(code.len(), 6, "code {code} at t={t} lost its padding");
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn code_is_stable_within_a_period_and_changes_across_it() {
        let key = decode_secret("JBSWY3DPEHPK3PXP").unwrap();
        assert_eq!(totp_at(&key, 100, 30, 6), totp_at(&key, 119, 30, 6));
        assert_ne!(totp_at(&key, 119, 30, 6), totp_at(&key, 120, 30, 6));
    }

    #[test]
    fn secret_decoding_tolerates_the_shapes_users_paste() {
        let canonical = decode_secret("JBSWY3DPEHPK3PXP").unwrap();
        for variant in [
            "jbswy3dpehpk3pxp",         // lowercase
            "JBSW Y3DP EHPK 3PXP",      // space-grouped
            "JBSW-Y3DP-EHPK-3PXP",      // hyphen-grouped
            "JBSWY3DPEHPK3PXP=",        // padded
            "  JBSWY3DPEHPK3PXP  ",     // surrounding whitespace
        ] {
            assert_eq!(decode_secret(variant).unwrap(), canonical, "variant: {variant}");
        }
    }

    #[test]
    fn rejects_input_that_is_not_a_secret() {
        // The most likely mistake is pasting a six-digit code instead of the secret.
        assert!(decode_secret("123456").is_err());
        assert!(decode_secret("").is_err());
        assert!(decode_secret("   ").is_err());
        assert!(decode_secret("not-base32!@#").is_err());
    }

    #[test]
    fn error_message_explains_the_likely_mistake() {
        let msg = TotpError::BadSecret.to_string();
        assert!(msg.contains("GARMIN_TOTP_SECRET"));
        assert!(msg.contains("not a six-digit code") || msg.contains("six-digit"));
    }
}
