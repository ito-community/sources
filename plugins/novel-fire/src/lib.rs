use ito_rs::export_novel_plugin;
use ito_rs::provider::NovelProvider;
use ito_rs::models::{ContentKind, FilterItem, Listing, Page, PageContent, HomeLayout, HomeComponent, HomeComponentValue};
use ito_rs::models::novel::{Novel, Chapter, PageResult, Status as NovelStatus, ContentRating};
use ito_rs::net::{NetResponse, Request};
use ito_rs::html::Node;
use ito_rs::{Error, Result};
use serde::Deserialize;

const BASE_URL: &str = "https://novelfire.net";

pub struct NovelFire;

/// Where Novel Fire now sends chapter readers: a loading page on this host, whose script points at the chapter
/// on `CHAPTER_MIRROR`. Only these hosts are followed, and only to a chapter page.
const REDIRECT_PAGE: &str = "https://readnovel.site/";
const CHAPTER_MIRROR: &str = "https://novelphoenix.com/novel/";

/// The body of a successful answer. Any other answer is an error, never an empty page: a rate-limit or error
/// page read as one would end a chapter list early or show up as a chapter's text.
fn success_body(res: NetResponse) -> Result<Vec<u8>> {
    match res.status {
        200..=299 => Ok(res.body),
        404 => Err(Error::NotFound("Novel Fire doesn't have this page anymore.".to_string())),
        429 => Err(Error::Server("Novel Fire is limiting requests. Try again in a minute.".to_string())),
        status => Err(Error::Server(format!("Novel Fire answered with HTTP {status}."))),
    }
}

/// The page at `url`, parsed.
fn fetch_page(url: &str) -> Result<Node> {
    Ok(Node::new(&success_body(Request::get(url).send()?)?))
}

/// A chapter's page. Novel Fire moves chapters to its mirror through a loading page: when the page has no
/// chapter text and points at the chapter on the mirror, the mirror's page is read instead.
fn chapter_page(url: &str) -> Result<Node> {
    let mut res = Request::get(url).send()?;
    // A host that doesn't follow a redirect to another site answers with the redirect itself.
    if (300..=399).contains(&res.status)
        && let Some(location) = header(&res, "location").filter(|l| l.starts_with(REDIRECT_PAGE))
    {
        res = Request::get(&location).send()?;
    }
    let body = success_body(res)?;
    let html = Node::new(&body);
    if !html.select("#content p")?.is_empty() {
        return Ok(html);
    }
    match mirror_target(&String::from_utf8_lossy(&body)) {
        Some(target) => fetch_page(&target),
        None => Ok(html),
    }
}

fn header(res: &NetResponse, name: &str) -> Option<String> {
    res.headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

/// The mirror chapter a loading page's script sends the reader to, without its query. `None` for any other page
/// or any other destination.
fn mirror_target(page: &str) -> Option<String> {
    let assignment = &page[page.find("window.location.href")?..];
    let quoted = &assignment[assignment.find('"')? + 1..];
    let url = quoted[..quoted.find('"')?].replace("\\/", "/");
    let url = url.split(['?', '#']).next()?.to_string();
    let path = url.strip_prefix(CHAPTER_MIRROR)?;
    let valid = !path.is_empty()
        && path.split('/').count() == 2
        && path.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'/');
    valid.then_some(url)
}

/// Builds one chapter from a row of the chapter list: the anchor's `href`, its `title`
/// attribute, and the row's chapter number text when the row has one.
fn chapter_from_row(href: Option<String>, title: &str, chapter_no: Option<&str>) -> Chapter {
    let key = href
        .as_deref()
        .map(|h| h.split('/').next_back().unwrap_or_default().to_string())
        .unwrap_or_default();
    Chapter {
        title: Some(title.split(':').nth(1).unwrap_or("").trim().to_string()),
        chapter: chapter_no.and_then(|n| n.trim().parse::<f64>().ok()),
        url: href,
        ..Chapter::new(key, ContentKind::Text)
    }
}

/// The site's numeric id for a series, from its page: chapter lists are fetched by it.
fn series_id(book: &Node) -> Result<Option<String>> {
    let Some(report) = book.select("#novel-report")?.into_iter().next() else {
        return Ok(None);
    };
    Ok(report
        .attr("report-post_id")?
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())))
}

/// Every chapter of the series, in one request, or `None` when the list can't be read as expected.
fn full_chapter_list(novel_key: &str, series_id: &str) -> Option<Vec<Chapter>> {
    let url = format!(
        "{BASE_URL}/ajax/listChapterDataAjax?draw=1\
         &columns%5B0%5D%5Bdata%5D=n_sort&columns%5B0%5D%5Bname%5D=cmm_posts_detail.n_sort\
         &columns%5B0%5D%5Bsearchable%5D=true&columns%5B0%5D%5Borderable%5D=true\
         &columns%5B0%5D%5Bsearch%5D%5Bvalue%5D=&columns%5B0%5D%5Bsearch%5D%5Bregex%5D=false\
         &order%5B0%5D%5Bcolumn%5D=0&order%5B0%5D%5Bdir%5D=asc&start=0&length=-1\
         &search%5Bvalue%5D=&search%5Bregex%5D=false&post_id={series_id}&only_bookmark=false"
    );
    let mut request = Request::get(&url);
    request.header("Accept", "application/json");
    request.header("X-Requested-With", "XMLHttpRequest");
    request.header("Referer", &format!("{BASE_URL}/book/{novel_key}"));
    let res = request.send().ok()?;
    if !(200..=299).contains(&res.status) {
        return None;
    }
    chapters_from_list(novel_key, &res.body)
}

#[derive(Deserialize)]
struct ChapterList {
    #[serde(rename = "recordsTotal")]
    records_total: usize,
    data: Vec<ChapterListRow>,
}

#[derive(Deserialize)]
struct ChapterListRow {
    n_sort: serde_json::Number,
    title: String,
}

/// The chapters in a chapter-list answer, built as the paged list builds its rows. `None` when the answer is not
/// the whole list (fewer rows than it counts, none at all) or a chapter has no whole number to address it by.
fn chapters_from_list(novel_key: &str, body: &[u8]) -> Option<Vec<Chapter>> {
    let list: ChapterList = serde_json::from_slice(body).ok()?;
    if list.data.is_empty() || list.data.len() != list.records_total {
        return None;
    }
    list.data
        .into_iter()
        .map(|row| {
            let number = row.n_sort.as_u64()?.to_string();
            let href = format!("/book/{novel_key}/chapter-{number}");
            Some(chapter_from_row(Some(href), &decode_entities(&row.title), Some(&number)))
        })
        .collect()
}

/// `text` with the HTML character references the chapter list escapes titles with decoded.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let name = &rest[1..end];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .map(|hex| u32::from_str_radix(hex, 16))
                    .or_else(|| name.strip_prefix('#').map(str::parse::<u32>))
                    .and_then(|n| n.ok())
                    .and_then(char::from_u32),
            }?;
            Some((ch, end))
        });
        match decoded {
            Some((ch, end)) => {
                out.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The chapters from the site's paged HTML list, 100 a page.
fn paged_chapter_list(novel_key: &str) -> Result<Vec<Chapter>> {
    let mut page = 1;
    let mut all_chapters = Vec::new();
    loop {
        let url = format!("{}/book/{}/chapters?page={}", BASE_URL, novel_key, page);
        let html = fetch_page(&url)?;
        let chapter_nodes = html.select(".chapter-list > li")?;
        if chapter_nodes.is_empty() {
            break;
        }
        for node in chapter_nodes {
            let a_node = node.select("a")?.into_iter().next();
            if let Some(a) = a_node {
                let title = a.attr("title")?.unwrap_or_default();
                let chap_no_node = node.select(".chapter-no")?.into_iter().next();
                let chapter_no = match chap_no_node {
                    Some(n) => Some(n.text()?),
                    None => None,
                };
                all_chapters.push(chapter_from_row(a.attr("href")?, &title, chapter_no.as_deref()));
            }
        }
        let has_next_page = html.select(".pagination li.page-item:last-child")?
            .into_iter()
            .next()
            .map(|node| node.attr("class").map(|c| !c.unwrap_or_default().contains("disabled")).unwrap_or(false))
            .unwrap_or(false);
        if !has_next_page {
            break;
        }
        page += 1;
    }
    Ok(all_chapters)
}

impl NovelProvider for NovelFire {

    fn get_home() -> Result<HomeLayout> {
        let listing1 = Listing { id: "overall-ranking".to_string(), name: "Newly Hotted Updates".to_string(), kind: 0 };
        let listing2 = Listing { id: "ratings".to_string(), name: "User Ratings".to_string(), kind: 0 };
        let listing3 = Listing { id: "most-lib".to_string(), name: "Most Bookmarked".to_string(), kind: 0 };
        let listing4 = Listing { id: "most-read".to_string(), name: "Most Read".to_string(), kind: 0 };
        let listing5 = Listing { id: "most-review".to_string(), name: "Most Reviewed".to_string(), kind: 0 };
        let listing6 = Listing { id: "most-comment".to_string(), name: "Most Commented".to_string(), kind: 0 };

        Ok(HomeLayout {
            components: vec![
                HomeComponent {
                    title: Some("Newly Hotted Updates".to_string()),
                    subtitle: Some("Hot updates from across the source!".to_string()),
                    value: HomeComponentValue::NovelScroller(
                        Self::get_novel_list(listing1.clone(), 1)?.entries,
                        Some(listing1),
                    ),
                },
                HomeComponent {
                    title: Some("User Ratings".to_string()),
                    subtitle: Some("Novels based on user ratings!".to_string()),
                    value: HomeComponentValue::NovelScroller(
                        Self::get_novel_list(listing2.clone(), 1)?.entries,
                        Some(listing2),
                    ),
                },
                HomeComponent {
                    title: Some("Most Bookmarked".to_string()),
                    subtitle: Some("Highest number of libraries".to_string()),
                    value: HomeComponentValue::NovelScroller(
                        Self::get_novel_list(listing3.clone(), 1)?.entries,
                        Some(listing3),
                    ),
                },
                HomeComponent {
                    title: Some("Most Read".to_string()),
                    subtitle: Some("Novels that are read the most".to_string()),
                    value: HomeComponentValue::NovelScroller(
                        Self::get_novel_list(listing4.clone(), 1)?.entries,
                        Some(listing4),
                    ),
                },
                HomeComponent {
                    title: Some("Most Reviewed".to_string()),
                    subtitle: Some("Novels with the most reviews".to_string()),
                    value: HomeComponentValue::NovelScroller(
                        Self::get_novel_list(listing5.clone(), 1)?.entries,
                        Some(listing5),
                    ),
                },
                HomeComponent {
                    title: Some("Most Commented".to_string()),
                    subtitle: Some("Novels with the most comments".to_string()),
                    value: HomeComponentValue::NovelScroller(
                        Self::get_novel_list(listing6.clone(), 1)?.entries,
                        Some(listing6),
                    ),
                },
            ],
        })
    }

    fn get_novel_list(listing: Listing, _page: i32) -> Result<PageResult> {
        let url = match listing.id.as_str() {
            "overall-ranking" => format!("{}/ranking", BASE_URL),
            "most-review" => format!("{}/ranking/most-review", BASE_URL),
            "most-lib" => format!("{}/ranking/most-lib", BASE_URL),
            "ratings" => format!("{}/ranking/ratings", BASE_URL),
            "most-read" => format!("{}/ranking/most-read", BASE_URL),
            "most-comment" => format!("{}/ranking/most-comment", BASE_URL),
            _ => format!("{}/ranking", BASE_URL),
        };

        let html = fetch_page(&url)?;
        let mut entries = Vec::new();
        
        let novel_nodes = html.select(".rank-novels .novel-item")?;
        for novel_node in novel_nodes {
            let a_nodes = novel_node.select("a")?;
            if let Some(a_first) = a_nodes.first() {
                let href = a_first.attr("href")?.unwrap_or_default();
                let key = href.replace("/book/", "");
                let url = format!("{}{}", BASE_URL, href);

                let cover = novel_node.select("img")?.into_iter().next()
                    .and_then(|img| img.attr("data-src").ok().flatten())
                    .map(|src| format!("{}{}", BASE_URL, src));

                let mut title = String::new();
                for a_node in a_nodes {
                    let txt = a_node.text()?.trim().to_string();
                    if !txt.is_empty() {
                        title = txt;
                    }
                    if title.is_empty()
                        && let Some(t) = a_node.attr("title")? {
                            title = t;
                        }
                    if !title.is_empty() {
                        break;
                    }
                }

                entries.push(Novel {
                    key,
                    title,
                    authors: None,
                    artist: None,
                    description: None,
                    tags: None,
                    cover,
                    url: Some(url),
                    status: NovelStatus::Ongoing,
                    content_rating: ContentRating::Safe,
                    nsfw: 0,
                    chapters: None,
                    catalog: None,
                });
            }
        }

        Ok(PageResult {
            entries,
            has_next_page: false,
        })
    }

    fn get_search_novel_list(query: &str, page: i32, _filters: Vec<FilterItem>) -> Result<PageResult> {
        let url = format!(
            "{}/search?keyword={}&page={}",
            BASE_URL,
            query,
            page
        );

        let html = fetch_page(&url)?;

        let mut entries = Vec::new();
        let novel_nodes = html.select(".novel-list.chapters > li.novel-item")?;
        
        for novel_node in novel_nodes {
            let a_node = novel_node.select("a")?.into_iter().next();
            if let Some(a) = a_node {
                let key = a.attr("href")?.unwrap_or_default().replace("/book/", "");
                let title = a.attr("title")?.unwrap_or_default();
                
                let img_node = novel_node.select("img")?.into_iter().next();
                let cover = img_node.and_then(|img| img.attr("src").ok().flatten()).map(|src| format!("{}/{}", BASE_URL, src));

                entries.push(Novel {
                    key,
                    title,
                    authors: None,
                    artist: None,
                    description: None,
                    tags: None,
                    cover,
                    url: None,
                    status: NovelStatus::Unknown,
                    content_rating: ContentRating::Safe,
                    nsfw: 0,
                    chapters: None,
                    catalog: None,
                });
            }
        }

        let has_next_page = html.select(".pagination li.page-item:last-child")?
            .into_iter()
            .next()
            .map(|node| node.attr("class").map(|c| !c.unwrap_or_default().contains("disabled")).unwrap_or(false))
            .unwrap_or(false);

        Ok(PageResult {
            entries,
            has_next_page,
        })
    }

    fn get_novel_update(novel: Novel, needs_details: bool, needs_chapters: bool) -> Result<Novel> {
        let mut updated_novel = novel.clone();

        let mut book = None;
        if needs_details {
            let url = format!("{}/book/{}", BASE_URL, novel.key);
            let html = fetch_page(&url)?;

            let cover_node = html.select(".cover img")?.into_iter().next();
            if let Some(n) = cover_node
                && let Some(src) = n.attr("src")? {
                    updated_novel.cover = Some(src);
                }

            let title_node = html.select(".main-head .novel-title")?.into_iter().next();
            if let Some(n) = title_node {
                updated_novel.title = n.text()?;
            }

            let author_node = html.select(".author a")?.into_iter().next();
            if let Some(n) = author_node
                && let Some(title) = n.attr("title")? {
                    updated_novel.authors = Some(vec![title]);
                }

            let desc_nodes = html.select("#info .content p")?;
            let mut desc_parts = Vec::new();
            for n in desc_nodes {
                desc_parts.push(n.text()?);
            }
            if !desc_parts.is_empty() {
                updated_novel.description = Some(desc_parts.join("\n\n"));
            }

            let tag_nodes = html.select(".categories ul li a")?;
            let mut tags = Vec::new();
            for n in tag_nodes {
                tags.push(n.text()?);
            }
            if !tags.is_empty() {
                updated_novel.tags = Some(tags);
            }

            updated_novel.status = NovelStatus::Ongoing;
            updated_novel.content_rating = ContentRating::Safe;
            updated_novel.url = Some(url);
            book = Some(html);
        }

        if needs_chapters {
            // The site's chapter list answers with every chapter at once; its paged HTML list is only the fallback
            // when that list can't be read. A long novel then takes two requests instead of one per 100 chapters.
            let listed = match book.as_ref() {
                Some(html) => series_id(html)?.and_then(|id| full_chapter_list(&novel.key, &id)),
                None => {
                    let html = fetch_page(&format!("{}/book/{}", BASE_URL, novel.key))?;
                    series_id(&html)?.and_then(|id| full_chapter_list(&novel.key, &id))
                }
            };
            updated_novel.chapters = Some(match listed {
                Some(chapters) => chapters,
                None => paged_chapter_list(&novel.key)?,
            });
        }

        Ok(updated_novel)
    }

    fn get_chapter_content(novel: Novel, chapter: Chapter) -> Result<Vec<Page>> {
        let url = format!("{}/book/{}/{}", BASE_URL, novel.key, chapter.key);
        let html = chapter_page(&url)?;

        let mut content_list = Vec::new();
        let p_nodes = html.select("#content p")?;
        if p_nodes.is_empty() {
            return Err(Error::Server("Novel Fire's page for this chapter has no chapter text.".to_string()));
        }

        for (index, node) in p_nodes.into_iter().enumerate() {
            let content = node.text()?;
            
            if content.starts_with('[') && content.ends_with(']') {
                let quote = content[1..content.len()-1].to_string();
                content_list.push(Page {
                    index: index as i32,
                    content: PageContent::Text(quote),
                    has_description: false,
                    description: None,
                    headers: None,
                });
            } else if content == "***" {
                content_list.push(Page {
                    index: index as i32,
                    content: PageContent::Text("---".to_string()), 
                    has_description: false,
                    description: None,
                    headers: None,
                });
            } else {
                content_list.push(Page {
                    index: index as i32,
                    content: PageContent::Text(content),
                    has_description: false,
                    description: None,
                    headers: None,
                });
            }
        }

        let current_len = content_list.len() as i32;
        content_list.push(Page {
            index: current_len,
            content: PageContent::Text("---".to_string()),
            has_description: false,
            description: None,
            headers: None,
        });

        let review_link = format!("LINK: [click here for chapter reviews.]({})", url);
        content_list.push(Page {
            index: current_len + 1,
            content: PageContent::Text(review_link),
            has_description: false,
            description: None,
            headers: None,
        });

        Ok(content_list)
    }
}

export_novel_plugin!(NovelFire);

#[cfg(test)]
mod tests {
    use super::{chapter_from_row, chapters_from_list, decode_entities, mirror_target};

    /// Chapter-list rows of a saved page, as (anchor href, anchor title, chapter number text).
    const CHAPTER_ROWS: &[(&str, &str, &str)] = &[
        (
            "/book/shadow-slave/chapter-1",
            "Chapter 1 - 1: Nightmare Begins",
            "1",
        ),
        (
            "/book/shadow-slave/chapter-2",
            "Chapter 2 - 2: Slave Caravan",
            "2",
        ),
        (
            "/book/shadow-slave/chapter-3",
            "Chapter 3 - 3: The Strings of Fate",
            "3",
        ),
        (
            "/book/shadow-slave/chapter-50",
            "Chapter 50 - 50: Death Trap",
            "50",
        ),
        (
            "/book/shadow-slave/chapter-100",
            "Chapter 100 - 100: Clear Conscience",
            "100",
        ),
    ];

    #[test]
    fn chapter_keys_are_stable() {
        let keys: Vec<String> = CHAPTER_ROWS
            .iter()
            .map(|(href, title, no)| chapter_from_row(Some(href.to_string()), title, Some(no)).key)
            .collect();
        assert_eq!(
            keys,
            [
                "chapter-1",
                "chapter-2",
                "chapter-3",
                "chapter-50",
                "chapter-100"
            ]
        );
    }

    /// The one-request list gives the same chapters as the paged list's rows, titles unescaped.
    #[test]
    fn the_full_list_builds_the_same_chapters_as_the_paged_rows() {
        let body = br#"{"draw":1,"recordsTotal":3,"recordsFiltered":3,"data":[
            {"n_sort":1,"slug":"chapter-1-1-nightmare-begins","title":"Chapter 1 - 1: Nightmare Begins"},
            {"n_sort":2,"slug":"chapter-2","title":"Chapter 2 - 2: Let&#039;s see &amp; explain"},
            {"n_sort":50,"slug":"chapter-50","title":"Chapter 50 - 50: Death Trap"}]}"#;
        let listed = chapters_from_list("shadow-slave", body).unwrap();
        let paged = [
            ("/book/shadow-slave/chapter-1", "Chapter 1 - 1: Nightmare Begins", "1"),
            ("/book/shadow-slave/chapter-2", "Chapter 2 - 2: Let's see & explain", "2"),
            ("/book/shadow-slave/chapter-50", "Chapter 50 - 50: Death Trap", "50"),
        ]
        .map(|(href, title, number)| chapter_from_row(Some(href.to_string()), title, Some(number)));
        assert_eq!(listed.len(), paged.len());
        for (a, b) in listed.iter().zip(paged.iter()) {
            assert_eq!(a.key, b.key);
            assert_eq!(a.title, b.title);
            assert_eq!(a.chapter, b.chapter);
            assert_eq!(a.url, b.url);
        }
        assert_eq!(listed[1].title.as_deref(), Some("Let's see & explain"));
    }

    /// Anything but the whole list sends the plugin back to the paged list.
    #[test]
    fn a_partial_or_unexpected_list_is_not_used() {
        let short = br#"{"recordsTotal":2334,"data":[{"n_sort":1,"title":"Chapter 1 - 1: A"}]}"#;
        assert!(chapters_from_list("x", short).is_none());
        assert!(chapters_from_list("x", br#"{"recordsTotal":0,"data":[]}"#).is_none());
        let fractional = br#"{"recordsTotal":1,"data":[{"n_sort":1.5,"title":"Chapter 1.5 - A"}]}"#;
        assert!(chapters_from_list("x", fractional).is_none());
        assert!(chapters_from_list("x", br#"{"message":"Server Error"}"#).is_none());
        assert!(chapters_from_list("x", b"<html>Too Many Requests</html>").is_none());
    }

    #[test]
    fn entities_decode_and_stray_ampersands_stay() {
        assert_eq!(decode_entities("A &amp; B &lt;C&gt; &quot;D&quot; &#039;E&#39; &#x2014; &apos;"), "A & B <C> \"D\" 'E' — '");
        assert_eq!(decode_entities("Tom & Jerry &unknown; &#xZZ;"), "Tom & Jerry &unknown; &#xZZ;");
    }

    /// The loading page Novel Fire sends chapter readers through points at the chapter on its mirror.
    #[test]
    fn the_loading_page_leads_only_to_a_chapter_on_the_mirror() {
        let page = r#"<html><head><title>Loading...</title></head><body><script>setTimeout(() => {
            window.location.href = "https:\/\/novelphoenix.com\/novel\/reverend-insanity\/chapter-1168?noti=1";
        }, 5);</script></body></html>"#;
        assert_eq!(
            mirror_target(page).as_deref(),
            Some("https://novelphoenix.com/novel/reverend-insanity/chapter-1168")
        );
        for elsewhere in [
            r#"window.location.href = "https://evil.example/novel/a/chapter-1";"#,
            r#"window.location.href = "https://novelphoenix.com.evil/novel/a/chapter-1";"#,
            r#"window.location.href = "https://novelphoenix.com/novel/a";"#,
            r#"window.location.href = "https://novelphoenix.com/novel/a/chapter-1/../../x";"#,
            "<html>no script</html>",
        ] {
            assert_eq!(mirror_target(elsewhere), None, "{elsewhere}");
        }
    }
}
