//! The page addresses of a chapter, as the reader component is given them.
//!
//! Each token is an obfuscated copy of one page's absolute image address. The reader decodes it
//! locally with a keystream that depends only on the token's zero-based position in its list and
//! on a seed the reader's script declares. No request, secret or expiry is involved.
//!
//! The seed is pinned in [`SEED`]. When it stops decoding the site's addresses, the source reads
//! the seed the current reader script declares instead (see [`seed_from_chunk`]).

/// The seed the reader's keystream starts from in the current build.
pub const SEED: &str = "di183bafh964gce5072";

/// The reader's multipliers for the keystream's starting state.
const POSITION_MULTIPLIER: u32 = 795_580_989;
const SEED_MULTIPLIER: u32 = 2_048_675_417;

/// The URL-safe base64 alphabet, without padding.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// How far into a reader module its seed declaration is searched. The declaration comes before
/// the module's text decoder, which is well inside this window in the current build.
const DECLARATION_WINDOW: usize = 600;
/// How far into a reader module its multiplier constants are searched; they sit about 4 KB in
/// the current build.
const MODULE_WINDOW: usize = 8 * 1024;

/// The page address of `token` at zero-based position `index` of its list, when the decoded bytes
/// are UTF-8 text. A wrong index or seed yields other bytes, which are not text.
pub fn decode(token: &str, index: usize, seed: &str) -> Option<String> {
    let stream = keystream(index, seed)?;
    let bytes: Vec<u8> = unscramble(token)
        .into_iter()
        .zip(stream)
        .map(|(byte, key)| byte ^ key)
        .collect();
    String::from_utf8(bytes).ok()
}

/// The key bytes for the token at `index`: one per token byte, from an endless stream.
fn keystream(index: usize, seed: &str) -> Option<impl Iterator<Item = u8>> {
    let mut state = initial_state(index, seed)?;
    Some(std::iter::repeat_with(move || {
        state = xorshift(state);
        // The key byte is the low byte of the state.
        state as u8
    }))
}

/// The keystream's starting state for the token at `index` under `seed`.
fn initial_state(index: usize, seed: &str) -> Option<u32> {
    let position = u32::try_from(index).ok()?.wrapping_add(1);
    let length = u32::try_from(seed.len()).ok()?;
    let mut state = position.wrapping_mul(POSITION_MULTIPLIER) ^ length;
    for byte in seed.bytes() {
        state = (state ^ u32::from(byte)).wrapping_mul(SEED_MULTIPLIER);
        state ^= state >> 15;
    }
    Some(if state == 0 { 1 } else { state })
}

/// One xorshift32 step.
fn xorshift(mut state: u32) -> u32 {
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    state
}

/// The bytes a token carries. Characters outside the alphabet are skipped, and bits left over
/// after the last whole byte are dropped.
fn unscramble(token: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(token.len() * 3 / 4);
    let mut bits = 0_u32;
    let mut count = 0_u32;
    for character in token.bytes() {
        let Some(value) = digit(character) else {
            continue;
        };
        bits = ((bits << 6) | value) & 0xFFFF;
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
        }
    }
    bytes
}

fn digit(character: u8) -> Option<u32> {
    ALPHABET
        .iter()
        .position(|&letter| letter == character)
        .and_then(|value| u32::try_from(value).ok())
}

/// The seed that the reader module `module` declares, read from the text of the reader script.
///
/// The module is found by its `<id>:function(` header. Within the module, the seed is the first
/// double-quoted literal of a seed's shape that comes before the module's text decoder, and both
/// multiplier constants of the keystream must appear in the module. Anything else is `None`. The
/// seed is a candidate only: the caller accepts it when it decodes every token.
pub fn seed_from_chunk(chunk: &str, module: u64) -> Option<String> {
    let marker = format!("{module}:function(");
    let start = chunk
        .match_indices(&marker)
        .find(|&(at, _)| at == 0 || matches!(chunk.as_bytes()[at - 1], b'{' | b','))?
        .0;
    let body = prefix(&chunk[start..], MODULE_WINDOW);
    if !body.contains(&POSITION_MULTIPLIER.to_string())
        || !body.contains(&SEED_MULTIPLIER.to_string())
    {
        return None;
    }
    let head = prefix(body, DECLARATION_WINDOW);
    let decoder = ["new TextEncoder", "new TextDecoder"]
        .iter()
        .filter_map(|name| head.find(name))
        .min()?;
    // Splitting on quotes puts the text inside each double-quoted literal at an odd position.
    head[..decoder]
        .split('"')
        .skip(1)
        .step_by(2)
        .find(|literal| plausible_seed(literal))
        .map(str::to_owned)
}

/// Whether `literal` has the shape of a seed: 8 to 64 ASCII letters or digits.
fn plausible_seed(literal: &str) -> bool {
    (8..=64).contains(&literal.len()) && literal.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// The first `max` bytes of `text`, or all of it when shorter, cut back to a character boundary.
fn prefix(text: &str, max: usize) -> &str {
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Encodes `address` as the reader's token for position `index` under `seed`. The inverse of
/// [`decode`], used to build fixtures.
#[cfg(test)]
pub(crate) fn scramble(address: &str, index: usize, seed: &str) -> String {
    let stream = keystream(index, seed).expect("position fits");
    let bytes: Vec<u8> = address
        .bytes()
        .zip(stream)
        .map(|(byte, key)| byte ^ key)
        .collect();
    let mut token = String::new();
    let (mut bits, mut count) = (0_u32, 0_u32);
    for byte in bytes {
        bits = (bits << 8) | u32::from(byte);
        count += 8;
        while count >= 6 {
            count -= 6;
            token.push(char::from(ALPHABET[((bits >> count) & 63) as usize]));
        }
    }
    if count > 0 {
        token.push(char::from(ALPHABET[((bits << (6 - count)) & 63) as usize]));
    }
    token
}

/// A reader script in the shape of the site's build: one module whose body declares `seed`
/// before its text decoder and holds both multipliers, beside an unrelated module.
#[cfg(test)]
pub(crate) fn reader_script(module: u64, seed: &str) -> String {
    format!(
        "(self.webpackChunk_N_E=self.webpackChunk_N_E||[]).push([[7],{{{module}:function(e,t,r){{\"use strict\";r.r(t);let d=\"{seed}\";new TextEncoder;let f=new TextDecoder(\"utf-8\",{{ignoreBOM:!0}}),g=795580989,h=2048675417;return e}},1:function(e){{e.exports=2}}}}]);"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live vectors: the token at a position, and the address the site shows for it.
    const VECTORS: [(&str, usize, &str); 6] = [
        (
            "spfvfINeTrkSh2H2W9rQJ8ntZ0At1dEaK47f7qJxaZWDgGFHwt8mu7UQsL9h-vxAvwlNmylq16a0NFq7j_mO2PNaF93xpevL_yC8C9hX2hH5yHrEJxAcJHha0hgS7ZSOwio72oowtK3RkT0yUjXD-qn5u4y4jeOrCw4",
            0,
            "https://media.templetoons.com/file/terms54/uploads/series/a-suckers-love-story/921c10d9-2732-4871-a6bc-dc154fe53561/01.jpg",
        ),
        (
            "jowy6N22UpdUNT4M7xFHrRKmt1ppePuv2dLbDh38LjRksvzKXjES9zoutCHy8MwGpOPdSNO89ht2pPQ7VNd0au0zCfU4rOfGfZYfGDLVqx-EZzZoV8sdwkZAekdB__4G_dSGP5rswgXGYZfQ6Yh04EdNoC_PBvLewWc",
            1,
            "https://media.templetoons.com/file/terms54/uploads/series/a-suckers-love-story/921c10d9-2732-4871-a6bc-dc154fe53561/02.jpg",
        ),
        (
            "FtC5aZ2qW3MdckM4qIFMllV_KYw81tTzqT4fEd40lrPol8r0-rqHwe6I7wkA5eOWHRs3-tilhe4KoIC5snowml9rpGVtE3leQTpK_jaWFoFxFbQdkRIpLnj0PqDhmN49xPbEDqWtHE89R_OXco93iQZGOz9ROpeW8Kc",
            2,
            "https://media.templetoons.com/file/terms54/uploads/series/a-suckers-love-story/921c10d9-2732-4871-a6bc-dc154fe53561/03.jpg",
        ),
        (
            "thVAvVAQzflXEVrP4a782jAzNjOZ9SmNtAvd_PuMTSsrMx1BKTTr_VyyN_3Gs4hRaOMHmTPWAGu-g3SIlTiNNlXXQ8iH6VvC49tWsJNwJmhVQrQtgUu6j4OD-oTFFPxM-u33MGJz9pP1f6jolI8Hg5rF0JMdBkCD_jQ",
            13,
            "https://media.templetoons.com/file/terms54/uploads/series/a-suckers-love-story/921c10d9-2732-4871-a6bc-dc154fe53561/14.jpg",
        ),
        (
            "spfvfINeTrkSh2H2W9rQJ8ntZ0At1dEaK47f7qJxaZWDgGFHwt8mu7UQsL9h-vxAvwlNmylq16a0NFr1iKGCxuIHU8eqrvjI6jflUJMcwQX_ySyTOhYTOyhA1Bxc7ZuNymI-2oo3tvmDjmJ2Uw",
            0,
            "https://media.templetoons.com/file/terms54/uploads/series/act-out/777def8d-4924-405f-b62b-56c4749ed6bd/01.jpg",
        ),
        (
            "9e89sk7tY6y5YO98_N-nWXi7kkx3M_54PF3Cnsc8_dkUxoiMElOlH-TYx5D8KSQ1Ega-pDO66E2IeU7iDOo3KJpzR4dnQcALOdfy3zIfkfd54WEPDDYMPvSNRMO0sQ9FJ25Ibqk2kDYDZ0m_MQ",
            5,
            "https://media.templetoons.com/file/terms54/uploads/series/act-out/777def8d-4924-405f-b62b-56c4749ed6bd/06.jpg",
        ),
    ];

    #[test]
    fn the_seed_state_matches_the_observed_values() {
        assert_eq!(initial_state(0, SEED), Some(0x0099_42F9));
        assert_eq!(initial_state(1, SEED), Some(0x6732_B0B4));
        assert_eq!(initial_state(2, SEED), Some(0x1D4F_667E));
        assert_eq!(initial_state(13, SEED), Some(0xC162_70A5));
    }

    #[test]
    fn the_bytes_and_keystream_match_the_observed_values() {
        let bytes = unscramble(VECTORS[0].0);
        assert_eq!(bytes.len(), 122);
        assert_eq!(bytes[..8], [0xb2, 0x97, 0xef, 0x7c, 0x83, 0x5e, 0x4e, 0xb9]);
        let key: Vec<u8> = keystream(0, SEED).expect("state").take(8).collect();
        assert_eq!(key, [0xda, 0xe3, 0x9b, 0x0c, 0xf0, 0x64, 0x61, 0x96]);
    }

    #[test]
    fn live_tokens_decode_to_the_addresses_the_site_shows() {
        for (token, index, address) in VECTORS {
            assert_eq!(
                decode(token, index, SEED).as_deref(),
                Some(address),
                "position {index}"
            );
        }
    }

    #[test]
    fn a_token_at_another_position_or_under_another_seed_is_not_its_address() {
        for (token, index, address) in VECTORS {
            assert_ne!(
                decode(token, index + 1, SEED).as_deref(),
                Some(address),
                "position {index} read one place later"
            );
            assert_ne!(
                decode(token, index, "wrongSeed000000").as_deref(),
                Some(address)
            );
        }
    }

    #[test]
    fn tokens_round_trip_through_the_encoder_for_any_text() {
        for (index, address) in [
            "https://media.templetoons.com/file/terms54/uploads/series/s/c/01.jpg",
            "https://media.templetoons.com/file/b/한글/02.webp?x=1",
            "a",
            "",
        ]
        .into_iter()
        .enumerate()
        {
            let token = scramble(address, index, "fixtureSeed2026");
            assert_eq!(
                decode(&token, index, "fixtureSeed2026").as_deref(),
                Some(address)
            );
        }
    }

    #[test]
    fn characters_outside_the_alphabet_are_skipped() {
        let (token, index, address) = VECTORS[0];
        let spaced = format!(" {} \n", &token[..40]) + &token[40..];
        assert_eq!(
            decode(&spaced, index, SEED).as_deref(),
            Some(address),
            "padding and line breaks carry no bytes"
        );
    }

    #[test]
    fn the_seed_is_read_from_the_declaring_reader_module() {
        let script = reader_script(44083, "fixtureSeed2026");
        assert_eq!(
            seed_from_chunk(&script, 44083).as_deref(),
            Some("fixtureSeed2026")
        );
        // Another module's id finds no module.
        assert_eq!(seed_from_chunk(&script, 4408), None);
        assert_eq!(seed_from_chunk(&script, 1), None);
    }

    #[test]
    fn a_module_without_the_decoder_or_its_constants_declares_no_seed() {
        let no_multipliers = reader_script(44083, "fixtureSeed2026").replace("g=795580989", "g=1");
        assert_eq!(seed_from_chunk(&no_multipliers, 44083), None);
        let no_decoder = reader_script(44083, "fixtureSeed2026")
            .replace("new TextEncoder;", "")
            .replace("new TextDecoder(\"utf-8\",{ignoreBOM:!0})", "0");
        assert_eq!(seed_from_chunk(&no_decoder, 44083), None);
        // A literal that is not seed-shaped is not a seed.
        let spaced = reader_script(44083, "not a seed");
        assert_eq!(seed_from_chunk(&spaced, 44083), None);
    }

    #[test]
    fn the_site_module_declares_the_pinned_seed() {
        // The reader's module as the current build declares it: the seed comes first, then the
        // decoder, with the multipliers further on.
        let module = format!(
            "44083:function(e,t,r){{\"use strict\";r.r(t);let d=\"{SEED}\";new TextEncoder;let f=new TextDecoder(\"utf-8\",{{ignoreBOM:!0}}),{}, {}}}",
            "p=()=>{}", "q=795580989,s=2048675417",
        );
        assert_eq!(seed_from_chunk(&module, 44083).as_deref(), Some(SEED));
    }
}
