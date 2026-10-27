//! MegaPlay sources for HiAnime's Ryu server, `s-2` route only.
//!
//! The caller fetches the embed page through HiAnime's `/stream/<token>/<lang>` hop, which
//! the host resolves to the MegaPlay page. This module reads the player id, asks getSources,
//! decrypts `enc` when present, and builds the playable video with its caption tracks.
//! Intro and outro data are not parsed: Ito's model has no field for them.

use crate::http::{self, Failure, Transport, USER_AGENT, Upstream};
use crate::markup;
use aes::cipher::{BlockModeDecrypt, KeyIvInit, block_padding::Pkcs7};
use base64::{
    Engine, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose::STANDARD},
};
use ito_rs::{
    Error,
    models::anime::{Subtitle, Video},
};
use std::collections::HashMap;

const ORIGIN: &str = "https://megaplay.buzz";
const ORIGIN_REFERER: &str = "https://megaplay.buzz/";

// getSources returns the stream as `enc`: AES-256-CBC with PKCS#7 padding, keyed by this
// 16-byte string zero-padded to 32 bytes, base64 (standard or URL-safe alphabet, padding
// optional). The plaintext is JSON `{"file": ...}`.
const SOURCE_KEY: &[u8; 16] = b"i?LMTAx0Q6,:}50U";
const SOURCE_IV: &[u8; 16] = b"W0;27ToaUpl_P%'c";

/// Largest caption file that is inlined. A larger track is dropped, not inlined.
const CAPTION_LIMIT: usize = 512 * 1024;

/// Most caption tracks one provider call inlines, across sub and dub. Later tracks are dropped.
const CAPTION_TRACK_BUDGET: usize = 12;

/// Most source caption bytes one provider call inlines, across sub and dub.
const CAPTION_BYTE_BUDGET: usize = 2 * 1024 * 1024;

/// Caption tracks resolved during one provider call. Sub and dub share their tracks, so each file
/// is fetched once, and the budgets cap what one call inlines across both languages.
#[derive(Default)]
pub struct CaptionCache {
    /// Track URL to its index in `inlined`. `None` marks a track that was not inlined.
    tracks: HashMap<String, Option<usize>>,
    /// Inlined WebVTT data URLs. Each is stored once and shared by index.
    inlined: Vec<String>,
    /// Source WebVTT bytes inlined so far.
    inlined_bytes: usize,
}

impl CaptionCache {
    pub fn new() -> Self {
        Self::default()
    }
}

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
/// `embed_url` is the `s-2` embed the hop resolved to, and it is the Referer for getSources.
/// `quality` is the label the caller shows for this stream. `captions` is shared by every
/// call of one provider request.
pub fn extract<N: Transport>(
    net: &N,
    embed_url: &str,
    embed_html: &str,
    lang: Lang,
    quality: &str,
    captions: &mut CaptionCache,
) -> Result<Vec<Video>, Failure> {
    let player_id = find_player_id(embed_html)
        .ok_or_else(|| Error::Html("MegaPlay player id not found".to_string()))?;
    let sources_url = format!(
        "{ORIGIN}/stream/getSources?id={player_id}&type={}",
        lang.as_str()
    );
    let response = http::get(
        net,
        Upstream::MegaPlay,
        &sources_url,
        &[
            ("Referer", embed_url),
            ("X-Requested-With", "XMLHttpRequest"),
        ],
    )?;
    let json: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|_| Error::Html("MegaPlay sources are not JSON".to_string()))?;
    let file =
        file_url(&json).ok_or_else(|| Error::Html("MegaPlay sources have no file".to_string()))?;
    let subtitles = subtitles(net, &json, captions)?;

    let headers = HashMap::from([
        ("Referer".to_string(), ORIGIN_REFERER.to_string()),
        ("Origin".to_string(), ORIGIN.to_string()),
        ("User-Agent".to_string(), USER_AGENT.to_string()),
    ]);
    Ok(vec![Video {
        url: file,
        quality: quality.to_string(),
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

/// Builds the caption tracks: `captions` only, each inlined as a WebVTT data URL when it passes
/// [`inline_caption`], followed by the "Off" entry that every stream carries.
///
/// A track that cannot be inlined is dropped. Its remote URL would need MegaPlay's Origin and
/// Referer, and `Subtitle` has no field to carry them.
fn subtitles<N: Transport>(
    net: &N,
    json: &serde_json::Value,
    captions: &mut CaptionCache,
) -> Result<Vec<Subtitle>, Failure> {
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
        let Some(index) = inline_caption(net, file, captions)? else {
            continue;
        };
        let language = track
            .get("label")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Unknown");
        subtitles.push(Subtitle {
            url: captions.inlined[index].clone(),
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

/// Returns the index of the track's data URL in `captions`, fetching its file at most once per
/// provider call.
///
/// `None` means the track is not inlined: its file was blocked or failed, is not WebVTT, is over
/// [`CAPTION_LIMIT`], or the call has spent its track or byte budget. Only that track is dropped.
/// A block on a caption does not fail the call.
fn inline_caption<N: Transport>(
    net: &N,
    file: &str,
    captions: &mut CaptionCache,
) -> Result<Option<usize>, Failure> {
    if let Some(known) = captions.tracks.get(file) {
        return Ok(*known);
    }
    if captions.inlined.len() >= CAPTION_TRACK_BUDGET {
        http::note("MegaPlay: caption track skipped, track budget reached");
        return Ok(None);
    }
    let body = match http::get(
        net,
        Upstream::MegaPlay,
        file,
        &[("Origin", ORIGIN), ("Referer", ORIGIN_REFERER)],
    ) {
        Ok(response) => Some(response.body),
        Err(Failure::Blocked { .. } | Failure::Other(_)) => None,
    };
    let inlined = body.and_then(|body| {
        let fits = captions.inlined_bytes + body.len() <= CAPTION_BYTE_BUDGET;
        let url = caption_data_url(&body).filter(|_| fits)?;
        Some((url, body.len()))
    });
    let index = match inlined {
        Some((url, bytes)) => {
            captions.inlined_bytes += bytes;
            captions.inlined.push(url);
            Some(captions.inlined.len() - 1)
        }
        None => {
            http::note("MegaPlay: caption track skipped");
            None
        }
    };
    captions.tracks.insert(file.to_string(), index);
    Ok(index)
}

/// Returns the WebVTT data URL for a caption body, or `None` when the body is not WebVTT or
/// is over [`CAPTION_LIMIT`].
fn caption_data_url(body: &[u8]) -> Option<String> {
    (body.len() <= CAPTION_LIMIT && is_webvtt(body)).then(|| vtt_data_url(body))
}

/// Returns true when the body starts with the WebVTT signature, after an optional UTF-8
/// byte-order mark. The signature must end at a space, tab, line break, or the end of the body.
fn is_webvtt(body: &[u8]) -> bool {
    let body = body.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(body);
    body.strip_prefix(b"WEBVTT").is_some_and(|rest| {
        rest.first()
            .is_none_or(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
    })
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
    use crate::fake::Canned;
    use aes::cipher::{BlockModeEncrypt, KeyIvInit};
    use serde_json::json;

    const EMBED_URL: &str = "https://megaplay.buzz/stream/s-2/2142/sub";
    const DUB_EMBED_URL: &str = "https://megaplay.buzz/stream/s-2/2142/dub";
    const EMBED_HTML: &str = r#"<html><div id="megaplay-player" data-id="2222"></div></html>"#;
    const SOURCES_SUB: &str = "https://megaplay.buzz/stream/getSources?id=2222&type=sub";
    const SOURCES_DUB: &str = "https://megaplay.buzz/stream/getSources?id=2222&type=dub";

    fn sources(tracks: serde_json::Value) -> String {
        json!({ "sources": { "file": "https://cdn.example/master.m3u8" }, "tracks": tracks })
            .to_string()
    }

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

    #[test]
    fn extract_uses_the_caller_quality_label_and_the_shared_playback_headers() {
        let net = Canned::new().on(SOURCES_SUB, 200, sources(json!([])));
        let videos = extract(
            &net,
            EMBED_URL,
            EMBED_HTML,
            Lang::Sub,
            "Auto SUB Ryu",
            &mut CaptionCache::new(),
        )
        .expect("sub stream");
        assert_eq!(videos.len(), 1);
        assert_eq!(videos[0].quality, "Auto SUB Ryu");
        let headers = videos[0].headers.as_ref().expect("playback headers");
        assert_eq!(
            headers.get("User-Agent").map(String::as_str),
            Some(USER_AGENT)
        );
        assert_eq!(
            headers.get("Origin").map(String::as_str),
            Some("https://megaplay.buzz")
        );
    }

    #[test]
    fn captions_are_fetched_once_and_only_valid_webvtt_within_the_cap_is_inlined() {
        let oversized = format!("WEBVTT\n{}", "x".repeat(CAPTION_LIMIT));
        let tracks = json!([
            { "kind": "captions", "file": "https://megaplay.buzz/cc/en.vtt", "label": "English" },
            { "kind": "captions", "file": "https://megaplay.buzz/cc/html.vtt", "label": "Html" },
            { "kind": "captions", "file": "https://megaplay.buzz/cc/big.vtt", "label": "Big" },
            { "kind": "captions", "file": "https://megaplay.buzz/cc/gone.vtt", "label": "Gone" },
            { "kind": "thumbnails", "file": "https://megaplay.buzz/cc/thumbs.vtt", "label": "Thumbs" }
        ]);
        let net = Canned::new()
            .on(SOURCES_SUB, 200, sources(tracks.clone()))
            .on(SOURCES_DUB, 200, sources(tracks))
            .on(
                "https://megaplay.buzz/cc/en.vtt",
                200,
                "WEBVTT\n\n00:00.000 --> 00:01.000\nHi\n",
            )
            .on(
                "https://megaplay.buzz/cc/html.vtt",
                200,
                "<html>not captions</html>",
            )
            .on("https://megaplay.buzz/cc/big.vtt", 200, oversized);
        let mut captions = CaptionCache::new();
        let sub = extract(
            &net,
            EMBED_URL,
            EMBED_HTML,
            Lang::Sub,
            "Auto SUB Ryu",
            &mut captions,
        )
        .expect("sub stream");
        let dub = extract(
            &net,
            DUB_EMBED_URL,
            EMBED_HTML,
            Lang::Dub,
            "Auto DUB Ryu",
            &mut captions,
        )
        .expect("dub stream");

        for video in sub.iter().chain(&dub) {
            let subs = video.subtitles.as_deref().expect("subtitles");
            let languages: Vec<&str> = subs.iter().map(|track| track.language.as_str()).collect();
            assert_eq!(languages, ["English", "Off"]);
            assert!(
                subs.iter()
                    .all(|track| track.url.starts_with("data:text/vtt"))
            );
        }
        for url in [
            "https://megaplay.buzz/cc/en.vtt",
            "https://megaplay.buzz/cc/html.vtt",
            "https://megaplay.buzz/cc/big.vtt",
            "https://megaplay.buzz/cc/gone.vtt",
        ] {
            assert_eq!(net.sent_to(url), 1, "{url} fetched more than once");
        }
        assert_eq!(net.sent_to("https://megaplay.buzz/cc/thumbs.vtt"), 0);
    }

    #[test]
    fn caption_requests_carry_the_megaplay_origin_referer_and_user_agent() {
        let net = Canned::new()
            .on(
                SOURCES_SUB,
                200,
                sources(json!([{ "kind": "captions", "file": "https://megaplay.buzz/cc/en.vtt", "label": "English" }])),
            )
            .on("https://megaplay.buzz/cc/en.vtt", 200, "WEBVTT\n");
        extract(
            &net,
            EMBED_URL,
            EMBED_HTML,
            Lang::Sub,
            "Auto SUB Ryu",
            &mut CaptionCache::new(),
        )
        .expect("sub stream");
        let request = net
            .requests()
            .into_iter()
            .find(|sent| sent.url == "https://megaplay.buzz/cc/en.vtt")
            .expect("caption request");
        assert_eq!(request.header("Origin"), Some("https://megaplay.buzz"));
        assert_eq!(request.header("Referer"), Some("https://megaplay.buzz/"));
        assert_eq!(request.header("User-Agent"), Some(USER_AGENT));
    }

    #[test]
    fn a_blocked_caption_drops_its_track_and_the_stream_still_plays() {
        let net = Canned::new()
            .on(
                SOURCES_SUB,
                200,
                sources(json!([{ "kind": "captions", "file": "https://megaplay.buzz/cc/en.vtt", "label": "English" }])),
            )
            .on("https://megaplay.buzz/cc/en.vtt", 403, "");
        let videos = extract(
            &net,
            EMBED_URL,
            EMBED_HTML,
            Lang::Sub,
            "Auto SUB Ryu",
            &mut CaptionCache::new(),
        )
        .expect("the stream still plays");
        let subs = videos[0].subtitles.as_deref().expect("subtitles");
        let languages: Vec<&str> = subs.iter().map(|track| track.language.as_str()).collect();
        assert_eq!(languages, ["Off"]);
    }

    #[test]
    fn tracks_past_the_track_budget_are_dropped_and_the_stream_still_plays() {
        let files: Vec<String> = (0..=CAPTION_TRACK_BUDGET)
            .map(|i| format!("https://megaplay.buzz/cc/t{i}.vtt"))
            .collect();
        let tracks: Vec<serde_json::Value> = files
            .iter()
            .enumerate()
            .map(|(i, file)| json!({ "kind": "captions", "file": file, "label": format!("Track {i}") }))
            .collect();
        let mut net = Canned::new().on(SOURCES_SUB, 200, sources(json!(tracks)));
        for file in &files {
            net = net.on(file, 200, "WEBVTT\n\n00:00.000 --> 00:01.000\nHi\n");
        }
        let videos = extract(
            &net,
            EMBED_URL,
            EMBED_HTML,
            Lang::Sub,
            "Auto SUB Ryu",
            &mut CaptionCache::new(),
        )
        .expect("the stream still plays");
        let subs = videos[0].subtitles.as_deref().expect("subtitles");
        assert_eq!(subs.len(), CAPTION_TRACK_BUDGET + 1, "tracks plus Off");
        assert_eq!(subs.last().map(|s| s.language.as_str()), Some("Off"));
        assert_eq!(net.sent_to(&files[CAPTION_TRACK_BUDGET]), 0);
    }

    #[test]
    fn tracks_past_the_byte_budget_are_dropped_and_the_stream_still_plays() {
        let body = format!("WEBVTT\n{}", "x".repeat(CAPTION_LIMIT - "WEBVTT\n".len()));
        let fitting = CAPTION_BYTE_BUDGET / CAPTION_LIMIT;
        let files: Vec<String> = (0..=fitting)
            .map(|i| format!("https://megaplay.buzz/cc/big{i}.vtt"))
            .collect();
        let tracks: Vec<serde_json::Value> = files
            .iter()
            .map(|file| json!({ "kind": "captions", "file": file, "label": "Big" }))
            .collect();
        let mut net = Canned::new().on(SOURCES_SUB, 200, sources(json!(tracks)));
        for file in &files {
            net = net.on(file, 200, body.clone());
        }
        let videos = extract(
            &net,
            EMBED_URL,
            EMBED_HTML,
            Lang::Sub,
            "Auto SUB Ryu",
            &mut CaptionCache::new(),
        )
        .expect("the stream still plays");
        let subs = videos[0].subtitles.as_deref().expect("subtitles");
        assert_eq!(
            subs.len(),
            fitting + 1,
            "tracks that fit the byte budget plus Off"
        );
    }

    #[test]
    fn a_caption_is_inlined_up_to_the_cap_and_no_further() {
        let mut at_cap = b"WEBVTT\n".to_vec();
        at_cap.resize(CAPTION_LIMIT, b'x');
        assert!(caption_data_url(&at_cap).is_some());
        at_cap.push(b'x');
        assert!(caption_data_url(&at_cap).is_none());
    }

    #[test]
    fn only_a_webvtt_signature_passes_validation() {
        assert!(is_webvtt(b"WEBVTT\n\n"));
        assert!(is_webvtt(b"WEBVTT"));
        assert!(is_webvtt(b"\xEF\xBB\xBFWEBVTT\r\n"));
        assert!(!is_webvtt(b"WEBVTTX"));
        assert!(!is_webvtt(b"<html>WEBVTT</html>"));
        assert!(!is_webvtt(b""));
    }
}
