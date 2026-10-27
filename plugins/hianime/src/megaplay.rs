//! MegaPlay sources for HiAnime's Ryu server, `s-2` route only.
//!
//! The caller fetches the embed page through HiAnime's `/stream/<token>/<lang>` hop, which
//! the host resolves to the MegaPlay page. This module reads the player id, asks getSources,
//! decrypts `enc` when present, and builds the playable video with its caption tracks.
//! Intro and outro data are not parsed: Ito's model has no field for them.

use crate::{http, markup};
use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use base64::{
    Engine, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose::STANDARD},
};
use ito_rs::{
    Error, Result,
    models::anime::{Subtitle, Video},
};
use std::collections::HashMap;

const ORIGIN: &str = "https://megaplay.buzz";
const ORIGIN_REFERER: &str = "https://megaplay.buzz/";
const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:133.0) Gecko/20100101 Firefox/133.0";

// getSources returns the stream as `enc`: AES-256-CBC with PKCS#7 padding, keyed by this
// 16-byte string zero-padded to 32 bytes, base64 (standard or URL-safe alphabet, padding
// optional). The plaintext is JSON `{"file": ...}`.
const SOURCE_KEY: &[u8; 16] = b"i?LMTAx0Q6,:}50U";
const SOURCE_IV: &[u8; 16] = b"W0;27ToaUpl_P%'c";

/// Audio language of a Ryu stream.
#[derive(Debug, Clone, Copy)]
pub enum Lang {
    Sub,
    Dub,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Sub, Lang::Dub];

    /// The language segment HiAnime and MegaPlay use in their paths.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sub => "sub",
            Self::Dub => "dub",
        }
    }

    /// The label shown in the quality name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Sub => "SUB",
            Self::Dub => "DUB",
        }
    }
}

/// Returns the MegaPlay embed URL for an episode on the `s-2` route.
pub fn embed_url(episode_id: &str, lang: Lang) -> String {
    format!("{ORIGIN}/stream/s-2/{episode_id}/{}", lang.as_str())
}

/// Resolves the playable Ryu stream from an already fetched MegaPlay embed page.
///
/// `embed_url` is the `s-2` embed the hop resolved to. It is the Referer for getSources.
pub fn extract(embed_url: &str, embed_html: &str, lang: Lang) -> Result<Vec<Video>> {
    let player_id = find_player_id(embed_html)
        .ok_or_else(|| Error::Html("MegaPlay player id not found".to_string()))?;
    let sources_url = format!(
        "{ORIGIN}/stream/getSources?id={player_id}&type={}",
        lang.as_str()
    );
    let response = http::fetch(
        &sources_url,
        &[
            ("User-Agent", USER_AGENT),
            ("Referer", embed_url),
            ("X-Requested-With", "XMLHttpRequest"),
        ],
    )?;
    let json: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|_| Error::Html("MegaPlay sources are not JSON".to_string()))?;
    let file =
        file_url(&json).ok_or_else(|| Error::Html("MegaPlay sources have no file".to_string()))?;
    let subtitles = subtitles(&json)?;

    let headers = HashMap::from([
        ("Referer".to_string(), ORIGIN_REFERER.to_string()),
        ("Origin".to_string(), ORIGIN.to_string()),
        ("User-Agent".to_string(), USER_AGENT.to_string()),
    ]);
    Ok(vec![Video {
        url: file,
        quality: "Auto".to_string(),
        headers: Some(headers),
        audio_tracks: None,
        subtitles: Some(subtitles),
    }])
}

/// Reads the numeric player id from the element with `id="megaplay-player"`.
///
/// Falls back to an element that carries both `data-id` and `data-realid`.
fn find_player_id(html: &str) -> Option<String> {
    let anchored = markup::tags(html)
        .find(|tag| tag.attr("id").as_deref() == Some("megaplay-player"))
        .and_then(|tag| tag.attr("data-id"));
    let fallback = markup::tags(html)
        .find(|tag| tag.attr("data-realid").is_some())
        .and_then(|tag| tag.attr("data-id"));
    numeric(anchored).or_else(|| numeric(fallback))
}

fn numeric(id: Option<String>) -> Option<String> {
    id.filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Returns the stream URL from `sources` (object or array) or from the encrypted `enc`.
fn file_url(json: &serde_json::Value) -> Option<String> {
    match json.get("sources") {
        Some(serde_json::Value::Array(sources)) => {
            sources.first()?.get("file")?.as_str().map(str::to_string)
        }
        Some(serde_json::Value::Object(source)) => source.get("file")?.as_str().map(str::to_string),
        _ => decrypt_source(json.get("enc")?.as_str()?),
    }
}

fn decrypt_source(enc: &str) -> Option<String> {
    let b64 = enc.replace('-', "+").replace('_', "/");
    let engine = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    let mut data = engine.decode(b64).ok()?;

    let mut key = [0u8; 32];
    key[..16].copy_from_slice(SOURCE_KEY);
    let plain = cbc::Decryptor::<aes::Aes256>::new(&key.into(), SOURCE_IV.into())
        .decrypt_padded::<Pkcs7>(&mut data)
        .ok()?;

    let json: serde_json::Value = serde_json::from_slice(plain).ok()?;
    json.get("file")?.as_str().map(str::to_string)
}

/// Builds the caption tracks: `captions` only, each inlined as a data URL when it can be
/// fetched, followed by the "Off" entry that every stream carries.
fn subtitles(json: &serde_json::Value) -> Result<Vec<Subtitle>> {
    let mut subtitles = Vec::new();
    let tracks = json
        .get("tracks")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten();
    for track in tracks {
        if track.get("kind").and_then(serde_json::Value::as_str) != Some("captions") {
            continue;
        }
        let Some(file) = track.get("file").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let language = track
            .get("label")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Unknown");
        subtitles.push(Subtitle {
            url: caption_url(file)?,
            language: language.to_string(),
            format: "vtt".to_string(),
            is_hardsub: false,
        });
    }
    subtitles.push(Subtitle {
        url: vtt_data_url(b"WEBVTT\n\n"),
        language: "Off".to_string(),
        format: "vtt".to_string(),
        is_hardsub: false,
    });
    Ok(subtitles)
}

/// Inlines a caption file as a data URL. If the file cannot be read, keeps its remote URL.
/// A block still fails the call.
fn caption_url(file: &str) -> Result<String> {
    match http::fetch(
        file,
        &[
            ("User-Agent", USER_AGENT),
            ("Origin", ORIGIN),
            ("Referer", ORIGIN_REFERER),
        ],
    ) {
        Ok(response) => Ok(vtt_data_url(&response.body)),
        Err(error) if http::is_blocked(&error) => Err(error),
        Err(_) => {
            ito_rs::host::print("MegaPlay: caption track kept as a remote URL");
            Ok(file.to_string())
        }
    }
}

fn vtt_data_url(body: &[u8]) -> String {
    format!(
        "data:text/vtt;charset=utf-8;base64,{}",
        STANDARD.encode(body)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::{BlockModeEncrypt, KeyIvInit};
    use serde_json::json;

    #[test]
    fn player_id_is_read_from_the_megaplay_player_element() {
        let html = concat!(
            r#"<div class="decoy" data-id="111"></div>"#,
            r#"<div id="megaplay-player" class="player" data-id="2222" data-x="y"></div>"#
        );
        assert_eq!(find_player_id(html).as_deref(), Some("2222"));
    }

    #[test]
    fn player_id_falls_back_to_an_element_with_data_realid() {
        let html = concat!(
            r#"<div class="other" data-id="999"></div>"#,
            r#"<div data-id="3333" data-realid="4444"></div>"#
        );
        assert_eq!(find_player_id(html).as_deref(), Some("3333"));
    }

    #[test]
    fn player_id_must_be_numeric() {
        let html = r#"<div id="megaplay-player" data-id="abc"></div>"#;
        assert_eq!(find_player_id(html), None);
    }

    #[test]
    fn embed_url_uses_the_s2_route() {
        assert_eq!(
            embed_url("2142", Lang::Dub),
            "https://megaplay.buzz/stream/s-2/2142/dub"
        );
        assert_eq!(
            embed_url("2142", Lang::Sub),
            "https://megaplay.buzz/stream/s-2/2142/sub"
        );
    }

    #[test]
    fn plain_sources_object_and_array_both_resolve_the_file() {
        let object = json!({ "sources": { "file": "https://cdn.example/object.m3u8" } });
        let array = json!({ "sources": [{ "file": "https://cdn.example/array.m3u8" }] });
        assert_eq!(
            file_url(&object).as_deref(),
            Some("https://cdn.example/object.m3u8")
        );
        assert_eq!(
            file_url(&array).as_deref(),
            Some("https://cdn.example/array.m3u8")
        );
    }

    #[test]
    fn enc_payload_decrypts_to_the_file_with_the_source_key_and_iv() {
        let plain = br#"{"file":"https://cdn.example/master.m3u8"}"#;
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(SOURCE_KEY);
        let mut buffer = vec![0u8; plain.len() + 16];
        buffer[..plain.len()].copy_from_slice(plain);
        let sealed = cbc::Encryptor::<aes::Aes256>::new(&key.into(), SOURCE_IV.into())
            .encrypt_padded::<Pkcs7>(&mut buffer, plain.len())
            .unwrap_or_else(|_| panic!("buffer has room for the padding block"));
        let enc = STANDARD.encode(sealed);

        let json = json!({ "enc": enc });
        assert_eq!(
            file_url(&json).as_deref(),
            Some("https://cdn.example/master.m3u8")
        );
    }
}
