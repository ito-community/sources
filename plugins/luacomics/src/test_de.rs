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

fn main() {
    let json = r#"{
  "meta": {
    "total": 2,
    "per_page": 20,
    "current_page": 1,
    "last_page": 1,
    "first_page": 1,
    "first_page_url": "/?page=1",
    "last_page_url": "/?page=1",
    "next_page_url": null,
    "previous_page_url": null
  },
  "data": [
    {
      "id": 131,
      "title": "The Snow Leopard Baby of the Black Leopard Family",
      "description": "...",
      "alternative_names": "흑표 가문의 설표 아기님",
      "series_type": "Comic",
      "series_slug": "the-snow-leopard-baby-of-the-black-leopard-family",
      "thumbnail": "https://media.luacomic.org/file/V4IKlhs/y1w5gjhyzpv0ydzfa4bqmndl.webp",
      "total_views": 56844,
      "status": "Ongoing",
      "created_at": "2025-01-25T13:28:48.496Z",
      "updated_at": "2026-04-17T19:14:17.277Z",
      "badge": "Comic",
      "rating": 5,
      "release_schedule": {
        "fri": true
      },
      "nu_link": null,
      "is_coming_soon": false,
      "is_pinned": false,
      "free_chapters": [],
      "paid_chapters": [],
      "latest_chapter": null,
      "meta": {
        "metadata": {},
        "chapters_count": "72"
      }
    }
  ]
}"#;
    let res: Result<SeriesResponse, _> = serde_json::from_str(json);
    println!("{:?}", res);
}
