#![allow(dead_code)]
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct ApiChapter {
    pub id: i32,
    pub chapter_name: String,
    pub chapter_slug: String,
    pub created_at: String,
    pub chapter_title: Option<String>,
    pub price: i32,
}

#[derive(Deserialize, Debug)]
pub struct SeriesResponse {
    pub data: Vec<ApiSeries>,
    pub meta: Meta,
}

#[derive(Deserialize, Debug)]
pub struct ApiSeries {
    pub id: i32,
    pub title: String,
    pub description: Option<String>,
    pub series_slug: String,
    pub thumbnail: String,
    pub status: String,
    pub tags: Option<Vec<String>>,
}

#[derive(Deserialize, Debug)]
pub struct Meta {
    pub current_page: i32,
    pub last_page: i32,
    pub total: i32,
}
