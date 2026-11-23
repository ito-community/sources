use ito_rs::{
    models::anime::{Subtitle, Video},
    net::{NetResponse, Request},
};
use std::collections::HashMap;
use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
use base64::Engine;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

// getSources returns the stream as `enc`: AES-256-CBC with PKCS#7 padding,
// keyed by this 16-byte string zero-padded to 32 bytes, base64 (standard or
// URL-safe alphabet, padding optional). The plaintext is JSON `{"file": ...}`.
const SOURCE_KEY: &[u8; 16] = b"i?LMTAx0Q6,:}50U";
const SOURCE_IV: &[u8; 16] = b"W0;27ToaUpl_P%'c";

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

/// The User-Agent every megaplay request sends.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:133.0) Gecko/20100101 Firefox/133.0";

/// The request for a megaplay embed page. `referer` is the site that embeds the player.
pub fn embed_request(embed_url: &str, referer: &str) -> Request {
    let mut request = Request::get(embed_url);
    request
        .header("User-Agent", USER_AGENT)
        .header("Referer", referer);
    request
}

/// The getSources URL for the player on an embed page. `None` when the embed failed or the page
/// has no player id.
pub fn sources_url(embed_url: &str, embed: &NetResponse) -> Option<String> {
    if embed.status != 200 {
        ito_rs::host::print(&format!("MegaPlay: Failed to fetch embed, status {}", embed.status));
        return None;
    }

    let html = String::from_utf8_lossy(&embed.body);
    let mut player_id = None;
    if let Some(start) = html.find("data-id=\"") {
        let after_start = &html[start + 9..];
        if let Some(end) = after_start.find('"') {
            player_id = Some(&after_start[..end]);
        }
    }
    let Some(player_id) = player_id else {
        ito_rs::host::print(&format!("MegaPlay: No player id in embed page: {}", embed_url));
        return None;
    };

    let mut api_url = format!("{}/stream/getSources?id={}", base_url(embed_url), player_id);
    let suffix = type_suffix(embed_url);
    if matches!(suffix, "sub" | "hsub" | "dub") {
        api_url.push_str(&format!("&type={}", suffix));
    }
    Some(api_url)
}

/// The request for a getSources URL. `embed_url` is its Referer.
pub fn sources_request(api_url: &str, embed_url: &str) -> Request {
    let mut request = Request::get(api_url);
    request
        .header("User-Agent", USER_AGENT)
        .header("Referer", embed_url)
        .header("X-Requested-With", "XMLHttpRequest");
    request
}

/// A playable stream read from its getSources response.
pub struct Stream {
    file_url: String,
    /// `(file, label)` for each caption track, in the order the response lists them.
    tracks: Vec<(String, String)>,
    base_url: String,
}

/// Reads the stream from a getSources response. `None` when the response failed or names no file.
pub fn stream(embed_url: &str, response: &NetResponse) -> Option<Stream> {
    if response.status != 200 {
        ito_rs::host::print(&format!("MegaPlay: API returned status {}", response.status));
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&response.body).ok()?;

    // Extract file url: plain `sources` (older responses) or encrypted `enc`
    let file_url = match json.get("sources") {
        Some(serde_json::Value::Array(sources)) => {
            sources.first().and_then(|s| s.get("file")).and_then(|f| f.as_str()).map(str::to_string)
        }
        Some(serde_json::Value::Object(sources)) => {
            sources.get("file").and_then(|f| f.as_str()).map(str::to_string)
        }
        _ => json.get("enc").and_then(|e| e.as_str()).and_then(decrypt_source),
    };
    let Some(file_url) = file_url else {
        ito_rs::host::print("MegaPlay: No file URL found in response.");
        return None;
    };

    // Hard-subbed streams have their captions in the picture, so there are no tracks to fetch.
    let tracks = if type_suffix(embed_url) == "hsub" {
        Vec::new()
    } else {
        caption_tracks(&json)
    };
    Some(Stream {
        file_url,
        tracks,
        base_url: base_url(embed_url).to_string(),
    })
}

/// The `(file, label)` of each caption or subtitle track, in order.
fn caption_tracks(json: &serde_json::Value) -> Vec<(String, String)> {
    json.get("tracks")
        .and_then(|t| t.as_array())
        .into_iter()
        .flatten()
        .filter_map(|track| {
            let kind = track.get("kind").and_then(|k| k.as_str())?;
            if kind != "captions" && kind != "subtitles" {
                return None;
            }
            let file = track.get("file").and_then(|f| f.as_str())?;
            let label = track.get("label").and_then(|l| l.as_str()).unwrap_or("Unknown");
            Some((file.to_string(), label.to_string()))
        })
        .collect()
}

/// The URL's last path segment, without its query: the stream type for sub, hsub and dub.
fn type_suffix(embed_url: &str) -> &str {
    let url_no_query = embed_url.split('?').next().unwrap_or(embed_url);
    url_no_query.rsplit('/').next().unwrap_or("")
}

/// The origin of an embed URL: everything before `/stream/`.
fn base_url(embed_url: &str) -> &str {
    embed_url.split("/stream/").next().unwrap_or("https://megaplay.buzz")
}

impl Stream {
    /// One request per caption track, in track order.
    pub fn caption_requests(&self) -> Vec<Request> {
        self.tracks
            .iter()
            .map(|(file, _)| {
                let mut request = Request::get(file);
                request
                    .header("User-Agent", USER_AGENT)
                    .header("Origin", &self.base_url)
                    .header("Referer", &format!("{}/", self.base_url));
                request
            })
            .collect()
    }

    /// Builds the video. `captions` yields one response per track, in track order: a 200 is inlined
    /// as WebVTT, and any other outcome keeps the track's remote URL.
    pub fn into_video(
        self,
        captions: &mut impl Iterator<Item = ito_rs::Result<NetResponse>>,
    ) -> Video {
        let mut subtitles = Vec::new();
        for (file, label) in self.tracks {
            let url = match captions.next() {
                Some(Ok(res)) if res.status == 200 => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&res.body);
                    format!("data:text/vtt;charset=utf-8;base64,{}", b64)
                }
                Some(Ok(res)) => {
                    ito_rs::host::print(&format!("MegaPlay: Failed to fetch VTT, status {}", res.status));
                    file
                }
                _ => file,
            };
            subtitles.push(Subtitle {
                url,
                language: label,
                format: "vtt".to_string(),
                is_hardsub: false,
            });
        }

        let empty_vtt = "WEBVTT\n\n";
        let empty_b64 = base64::engine::general_purpose::STANDARD.encode(empty_vtt);
        subtitles.push(Subtitle {
            url: format!("data:text/vtt;charset=utf-8;base64,{}", empty_b64),
            language: "Off".to_string(),
            format: "vtt".to_string(),
            is_hardsub: false,
        });

        let mut headers = HashMap::new();
        headers.insert("Referer".to_string(), format!("{}/", self.base_url));
        headers.insert("Origin".to_string(), self.base_url.clone());
        headers.insert("User-Agent".to_string(), USER_AGENT.to_string());

        Video {
            url: self.file_url,
            quality: "Auto".to_string(), // we can fetch m3u8 and parse qualities if we want, but "Auto" is often fine for HLS
            headers: Some(headers),
            audio_tracks: None,
            subtitles: Some(subtitles),
        }
    }
}
