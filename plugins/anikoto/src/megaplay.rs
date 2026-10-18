use ito_rs::{
    net::Request,
    models::anime::{Video, Subtitle},
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

pub fn extract(embed_url: &str, referer: &str) -> Option<Vec<Video>> {
    let user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:133.0) Gecko/20100101 Firefox/133.0";
    
    // 1. Fetch embed page
    let res = Request::get(embed_url)
        .header("User-Agent", user_agent)
        .header("Referer", referer)
        .send().ok()?;

    if res.status != 200 {
        ito_rs::host::print(&format!("MegaPlay: Failed to fetch embed, status {}", res.status));
        return None;
    }

    let html = String::from_utf8_lossy(&res.body);

    // 2. Extract player ID
    let mut player_id = None;
    if let Some(start) = html.find("data-id=\"") {
        let after_start = &html[start + 9..];
        if let Some(end) = after_start.find('"') {
            player_id = Some(&after_start[..end]);
        }
    }
    
    let player_id = match player_id {
        Some(id) => id,
        None => {
            ito_rs::host::print(&format!("MegaPlay: No player id in embed page: {}", embed_url));
            return None;
        }
    };

    // 3. Fetch sources JSON
    let base_url = embed_url.split("/stream/").next().unwrap_or("https://megaplay.buzz");
    
    let url_no_query = embed_url.split('?').next().unwrap_or(embed_url);
    let type_suffix = url_no_query.split('/').last().unwrap_or("");
    
    let mut api_url = format!("{}/stream/getSources?id={}", base_url, player_id);
    if type_suffix == "sub" || type_suffix == "hsub" || type_suffix == "dub" {
        api_url.push_str(&format!("&type={}", type_suffix));
    }

    let api_res = Request::get(&api_url)
        .header("User-Agent", user_agent)
        .header("Referer", embed_url)
        .header("X-Requested-With", "XMLHttpRequest")
        .send().ok()?;

    if api_res.status != 200 {
        ito_rs::host::print(&format!("MegaPlay: API returned status {}", api_res.status));
        return None;
    }

    let json: serde_json::Value = serde_json::from_slice(&api_res.body).ok()?;

    let mut videos = Vec::new();
    
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

    let file_url = match file_url {
        Some(u) => u,
        None => {
            ito_rs::host::print("MegaPlay: No file URL found in response.");
            return None;
        }
    };

    // Extract subtitles
    let mut subtitles = Vec::new();
    if type_suffix != "hsub" {
        if let Some(tracks) = json.get("tracks").and_then(|t| t.as_array()) {
        let base_url = embed_url.split("/stream/").next().unwrap_or("https://megaplay.buzz");
        for track in tracks {
            if let Some(kind) = track.get("kind").and_then(|k| k.as_str()) {
                if kind == "captions" || kind == "subtitles" {
                    if let Some(file) = track.get("file").and_then(|f| f.as_str()) {
                        let label = track.get("label").and_then(|l| l.as_str()).unwrap_or("Unknown");
                        
                        let mut final_url = file.to_string();
                        if let Ok(vtt_res) = Request::get(file)
                            .header("User-Agent", user_agent)
                            .header("Origin", base_url)
                            .header("Referer", &format!("{}/", base_url))
                            .send()
                        {
                            if vtt_res.status == 200 {
                                let b64 = base64::engine::general_purpose::STANDARD.encode(&vtt_res.body);
                                final_url = format!("data:text/vtt;charset=utf-8;base64,{}", b64);
                            } else {
                                ito_rs::host::print(&format!("MegaPlay: Failed to fetch VTT, status {}", vtt_res.status));
                            }
                        }

                        subtitles.push(Subtitle {
                            url: final_url,
                            language: label.to_string(),
                            format: "vtt".to_string(),
                            is_hardsub: false,
                        });
                    }
                }
            }
        }
        }
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
    let base_url = embed_url.split("/stream/").next().unwrap_or("https://megaplay.buzz");
    headers.insert("Referer".to_string(), format!("{}/", base_url));
    headers.insert("Origin".to_string(), base_url.to_string());
    headers.insert("User-Agent".to_string(), user_agent.to_string());

    videos.push(Video {
        url: file_url,
        quality: "Auto".to_string(), // we can fetch m3u8 and parse qualities if we want, but "Auto" is often fine for HLS
        headers: Some(headers),
        audio_tracks: None,
        subtitles: if subtitles.is_empty() { None } else { Some(subtitles) },
    });

    Some(videos)
}
