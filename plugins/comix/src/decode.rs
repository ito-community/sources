//! Decoding of sealed responses: a `{"e": "<base64>"}` body with `x-enc: 2` (spec section 3.2).

use crate::key::KEY_MATERIAL;
use base64::Engine;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// Accepts either base64 alphabet and padded or unpadded input, as the site's client does.
const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

/// Offset of the 1024-byte table inside the key material.
const TABLE_START: usize = 64;

/// Why a sealed body could not be decoded.
#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The text is not base64 in either alphabet.
    Base64,
    /// The payload is shorter than its two-byte rotation header and a byte of data.
    TooShort,
}

/// Decodes the `e` text of a sealed body into the plaintext JSON bytes.
pub fn decode(envelope: &str) -> Result<Vec<u8>, DecodeError> {
    let standard: String = envelope
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            other => other,
        })
        .collect();
    let payload = LENIENT.decode(standard).map_err(|_| DecodeError::Base64)?;
    if payload.len() < 4 {
        return Err(DecodeError::TooShort);
    }

    // The first two bytes name a rotation of the table, so the same request decodes the same way.
    let rotation = ((usize::from(payload[0]) << 8) | usize::from(payload[1])) & 1023;
    Ok(payload[2..]
        .iter()
        .enumerate()
        .map(|(index, byte)| byte ^ KEY_MATERIAL[TABLE_START + ((index + rotation) & 1023)])
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    /// Sealed `GET /manga/xw6nq` body, spec section 3.2: the envelope text of the live response.
    const XW6NQ_ENVELOPE: &str = include_str!("../tests/fixtures/xw6nq-envelope.txt");

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn live_envelope_decodes_to_its_recorded_plaintext() {
        let plain = decode(XW6NQ_ENVELOPE).expect("live envelope decodes");
        assert_eq!(plain.len(), 1072);
        assert_eq!(
            hex(&Sha256::digest(&plain)),
            "97fa20ddeccbf5fd4803547f462c2b045262e2dc5140d40cdd9f55eb5b40d8d2"
        );
        assert!(plain.starts_with(br#"{"status":"ok","result":{"id":146119,"hid":"xw6nq""#));
    }

    #[test]
    fn url_safe_alphabet_decodes_the_same_bytes() {
        let url_safe = XW6NQ_ENVELOPE.replace('+', "-").replace('/', "_");
        assert_eq!(decode(&url_safe), decode(XW6NQ_ENVELOPE));
    }

    #[test]
    fn payload_shorter_than_its_header_is_refused() {
        // Three bytes of payload: "AAAA" decodes to 3 bytes.
        assert_eq!(decode("AAAA"), Err(DecodeError::TooShort));
    }

    #[test]
    fn text_that_is_not_base64_is_refused() {
        assert_eq!(decode("not base64!"), Err(DecodeError::Base64));
    }
}
