//! nhentai API v2. Public reads work without a key. A saved key is sent for a
//! higher limit and for favorites, and a rejected key falls back on public calls.

use crate::query::Sort;
use ito_rs::defaults;
use ito_rs::net::Request;
use ito_rs::{Error, Result};
use serde::Deserialize;
use serde::de::DeserializeOwned;

const API: &str = "https://nhentai.net/api/v2";
const USER_AGENT: &str = "nhentai/1.0.0 (https://github.com/ito-community/sources)";
const REFERER: &str = "https://nhentai.net/";

#[derive(Debug, Deserialize)]
pub struct ListItem {
    pub id: i64,
    #[serde(default)]
    pub english_title: String,
    #[serde(default)]
    pub japanese_title: Option<String>,
    #[serde(default)]
    pub thumbnail: String,
    #[serde(default)]
    pub num_pages: i32,
    #[serde(default)]
    pub num_favorites: i32,
    #[serde(default)]
    pub tag_ids: Vec<i64>,
    #[serde(default)]
    pub blacklisted: bool,
}

#[derive(Debug, Deserialize)]
pub struct Page {
    pub result: Vec<ListItem>,
    pub num_pages: i32,
}

#[derive(Debug, Deserialize)]
pub struct Gallery {
    pub id: i64,
    pub title: Title,
    pub cover: ImageFile,
    #[serde(default)]
    pub scanlator: String,
    pub upload_date: i64,
    #[serde(default)]
    pub tags: Vec<Tag>,
    pub num_pages: i32,
    pub num_favorites: i32,
    #[serde(default)]
    pub pages: Vec<PageImage>,
}

#[derive(Debug, Deserialize)]
pub struct Title {
    #[serde(default)]
    pub english: String,
    #[serde(default)]
    pub japanese: Option<String>,
    #[serde(default)]
    pub pretty: String,
}

#[derive(Debug, Deserialize)]
pub struct ImageFile {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct Tag {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub count: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PageImage {
    #[serde(default)]
    pub number: i32,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct TagPage {
    pub result: Vec<Tag>,
    pub num_pages: i32,
    #[serde(default)]
    pub total: Option<i32>,
}

#[derive(Deserialize)]
struct RandomId {
    id: i64,
}

struct Call {
    url: String,
    method: &'static str,
    body: Option<Vec<u8>>,
}

pub fn api_key() -> Option<String> {
    let key = defaults::get("api_key").ok().flatten()?;
    let key = key.trim();
    (!key.is_empty()).then(|| key.to_owned())
}

pub fn search(query: &str, page: i32, sort: Sort) -> Result<Page> {
    let url = format!(
        "{API}/search?query={}&page={}&sort={}",
        encode(query),
        page.max(1),
        sort.as_str()
    );
    get(&url)
}

pub fn gallery(id: i64) -> Result<Gallery> {
    get(&format!("{API}/galleries/{id}"))
}

pub fn random_id() -> Result<i64> {
    let body: RandomId = get(&format!("{API}/galleries/random"))?;
    if body.id <= 0 {
        return Err(Error::Server("nhentai did not return a gallery".into()));
    }
    Ok(body.id)
}

pub fn favorites(page: i32) -> Result<Page> {
    if api_key().is_none() {
        return Err(Error::NeedsSetup(
            "Add an nhentai API key in source settings to open favorites.".into(),
        ));
    }
    let url = format!("{API}/favorites?page={}", page.max(1));
    execute(
        &Call {
            url,
            method: "GET",
            body: None,
        },
        false,
    )
    .and_then(decode)
}

pub fn search_tags(kind: &str, query: &str, limit: u32) -> Result<Vec<Tag>> {
    let body = serde_json::to_vec(&serde_json::json!({
        "type": kind,
        "query": query,
        "limit": limit.clamp(1, 50),
    }))
    .map_err(|_| Error::Server("could not encode the nhentai request".into()))?;
    execute(
        &Call {
            url: format!("{API}/tags/search"),
            method: "POST",
            body: Some(body),
        },
        true,
    )
    .and_then(decode)
}

pub fn tags_by_type(kind: &str, page: i32, per_page: i32) -> Result<TagPage> {
    let url = format!(
        "{API}/tags/{}?sort=popular&page={}&per_page={}",
        encode(kind),
        page.max(1),
        per_page.clamp(1, 100)
    );
    get(&url)
}

pub fn account_accepted() -> Result<bool> {
    if api_key().is_none() {
        return Ok(false);
    }
    let response = execute(
        &Call {
            url: format!("{API}/user"),
            method: "GET",
            body: None,
        },
        false,
    )?;
    match response.status {
        200..=299 => Ok(true),
        401 => Ok(false),
        status => Err(status_error(status)
            .unwrap_or_else(|| Error::Server(format!("nhentai returned HTTP {status}")))),
    }
}

fn get<T: DeserializeOwned>(url: &str) -> Result<T> {
    execute(
        &Call {
            url: url.to_owned(),
            method: "GET",
            body: None,
        },
        true,
    )
    .and_then(decode)
}

fn execute(call: &Call, fallback: bool) -> Result<ito_rs::net::NetResponse> {
    let response = dispatch(call, true)?;
    if response.status == 401 && fallback && api_key().is_some() {
        let retry = dispatch(call, false)?;
        if (200..300).contains(&retry.status) {
            return Ok(retry);
        }
        return Err(Error::Unauthorized("nhentai rejected the API key".into()));
    }
    if let Some(err) = status_error(response.status) {
        return Err(err);
    }
    Ok(response)
}

fn dispatch(call: &Call, authorize: bool) -> Result<ito_rs::net::NetResponse> {
    let mut request = Request::new(&call.url, call.method);
    request.header("User-Agent", USER_AGENT);
    request.header("Accept", "application/json");
    request.header("Referer", REFERER);
    let key = authorize.then(api_key).flatten();
    if let Some(key) = &key {
        request.header("Authorization", &format!("Key {key}"));
    }
    if let Some(body) = &call.body {
        request.header("Content-Type", "application/json");
        request.body(body);
    }
    request.send()
}

fn decode<T: DeserializeOwned>(response: ito_rs::net::NetResponse) -> Result<T> {
    serde_json::from_slice(&response.body)
        .map_err(|_| Error::Server("nhentai returned data this source could not read".into()))
}

fn status_error(status: i32) -> Option<Error> {
    match status {
        200..=299 => None,
        401 => Some(Error::Unauthorized("nhentai rejected the API key".into())),
        404 => Some(Error::NotFound("nhentai gallery was not found".into())),
        429 => Some(Error::Server(
            "nhentai rate limit exceeded. Wait about a minute and try again.".into(),
        )),
        _ => Some(Error::Server(format!("nhentai returned HTTP {status}"))),
    }
}

fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
