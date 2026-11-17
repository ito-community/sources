//! The `_` request token (spec section 2.2).
//!
//! The token is HMAC-SHA-256 keyed by the two midstates in the key material: the states reached
//! after compressing the inner and outer pad blocks. The raw key is never present, so the MAC is
//! computed by resuming SHA-256 from each state.

use crate::key::{KEY_ID, KEY_MATERIAL};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::compress256;
use sha2::digest::consts::U64;
use sha2::digest::generic_array::GenericArray;

/// Bytes of the tag kept in the token: 12 bytes encode to the 16 characters the site expects.
const TAG_PREFIX_BYTES: usize = 12;

/// Length of the single block the pad states already account for.
const PAD_BLOCK_BYTES: usize = 64;

/// Returns the `_` token for a canonical message (see [`crate::query::Query::message`]).
pub fn token(message: &str) -> String {
    let tag = tag(message.as_bytes());
    format!(
        "{KEY_ID}.{}",
        URL_SAFE_NO_PAD.encode(&tag[..TAG_PREFIX_BYTES])
    )
}

/// The full 32-byte MAC of `message`.
fn tag(message: &[u8]) -> [u8; 32] {
    let inner = resume_sha256(&KEY_MATERIAL[0..32], message);
    resume_sha256(&KEY_MATERIAL[32..64], &inner)
}

/// SHA-256 continued from `state` (eight big-endian words), as if one pad block had already been
/// hashed. The padded length counts that block.
fn resume_sha256(state: &[u8], message: &[u8]) -> [u8; 32] {
    let mut words = state_words(state);

    let bit_length = (PAD_BLOCK_BYTES as u64 + message.len() as u64) * 8;
    let mut padded = Vec::with_capacity(message.len() + 72);
    padded.extend_from_slice(message);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());

    let blocks: Vec<GenericArray<u8, U64>> = padded
        .as_chunks::<64>()
        .0
        .iter()
        .map(|block| GenericArray::clone_from_slice(block))
        .collect();
    compress256(&mut words, &blocks);

    let mut out = [0u8; 32];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(words) {
        *chunk = word.to_be_bytes();
    }
    out
}

fn state_words(bytes: &[u8]) -> [u32; 8] {
    let mut words = [0u32; 8];
    for (word, chunk) in words.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *word = u32::from_be_bytes(*chunk);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Live vectors from spec section 2.3: message, full tag, and token. Each token was accepted by
    /// the server, or produced by the site's own client where the row says so.
    const VECTORS: &[(&str, &str, &str)] = &[
        (
            "/manga/8v88/chapters?limit=20&order[number]=desc&page=1&v=0&w=0",
            "46b9ad263bbc29b637bc2eb276ccb6b61efbd438ea7e664addfa4fe3a8fe44e2",
            "etu.RrmtJju8KbY3vC6y",
        ),
        (
            "/manga?content_rating[0]=safe&content_rating[1]=suggestive&keyword=solo&limit=28&order[chapter_updated_at]=desc&page=1",
            "ac5f05f2895a6c0243833621004f0750be959d02e0ee5dda948d4fee1fed3a8d",
            "etu.rF8F8olabAJDgzYh",
        ),
        (
            "/manga?content_rating[0]=safe&content_rating[1]=suggestive&limit=28&order[created_at]=desc&page=2",
            "4d2e3628d21f9cb20b455ab2808a2882df10128d668aca10d409eb0b883071b5",
            "etu.TS42KNIfnLILRVqy",
        ),
        (
            "/manga/8v88/chapters?limit=20&order[number]=desc&page=1",
            "3370a0302f32c5e3176efe771841358aa8ef46a881520cc5c9345a586e649162",
            "etu.M3CgMC8yxeMXbv53",
        ),
        (
            "/manga/8v88",
            "d30da5cd8942656e4f8a5d26b17f46e53de81391952118e1b3785482b3975e65",
            "etu.0w2lzYlCZW5Pil0m",
        ),
        (
            "/manga/8v88?v=0&w=0",
            "5ceb3f26d54e2bc54b53391b72e8ea81c71e8df504f5989732e043542e854d57",
            "etu.XOs_JtVOK8VLUzkb",
        ),
        (
            "/chapters/11734794?v=0&w=0",
            "707764fa1a22610ad691ea5620a6ab60638c36b91dc7315c78a2cc4867acfc6a",
            "etu.cHdk-hoiYQrWkepW",
        ),
        (
            "/manga/top?content_rating[0]=safe&content_rating[1]=suggestive&days=1&limit=2&type=trending",
            "ba8f4b4d024746e66f847db3cabd815e4e7198954a9f21b92579540e558aaa18",
            "etu.uo9LTQJHRuZvhH2z",
        ),
        (
            "/manga?keyword=solo leveling&limit=2&page=1",
            "b3f83d98a62bfc1772b630c4b51d670e4ac6ffeab07c88e4a5eb05f24b6f3482",
            "etu.s_g9mKYr_BdytjDE",
        ),
        (
            "/manga?keyword=日本語 & a=b&limit=1&page=1",
            "058ed3919a8a6a4dc95db7c5ebdc90d3075353559ee49a4cf90c50be8808bf68",
            "etu.BY7TkZqKak3JXbfF",
        ),
    ];

    #[test]
    fn tags_match_the_live_vectors() {
        for (message, expected_tag, _) in VECTORS {
            assert_eq!(
                hex(&tag(message.as_bytes())),
                *expected_tag,
                "message {message}"
            );
        }
    }

    #[test]
    fn tokens_match_the_live_vectors() {
        for (message, _, expected_token) in VECTORS {
            assert_eq!(token(message), *expected_token, "message {message}");
        }
    }
}
