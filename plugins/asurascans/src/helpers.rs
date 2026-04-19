pub const BASE_URL: &str = "https://asurascans.com";

/// Returns the ID of a manga from a URL.
pub fn get_manga_key(url: &str) -> Option<String> {
	// Asura Scans appends a random string at the end of each series slug
	// The random string is not necessary, along with the trailing '-'

	// remove query parameters
	let path = url.split('?').next().unwrap_or("");

	// find the segment after "series"
	let manga_segment = path
		.split('/')
		.skip_while(|segment| *segment != "comics")
		.nth(1)?;

	// find the last '-' and keep it in the id
	let pos = manga_segment.rfind('-')?;
	Some(manga_segment[..pos].into())
}

/// Returns the ID of a chapter from a URL.
pub fn get_chapter_key(url: &str) -> Option<String> {
	// remove query parameters
	let path = url.split('?').next().unwrap_or("");

	// find the segment after "chapter"
	let chapter_segment = path
		.split('/')
		.skip_while(|segment| *segment != "chapter")
		.nth(1)?;

	// extract only the numeric (and '.') prefix
	let end_pos = chapter_segment
		.find(|c: char| !c.is_numeric() && c != '.')
		.unwrap_or(chapter_segment.len());

	Some(chapter_segment[..end_pos].into())
}

/// Returns full URL of a manga from a manga ID.
pub fn get_manga_url(manga_id: &str) -> String {
	format!("{BASE_URL}/comics/{manga_id}")
}

/// Returns full URL of a chapter from a chapter ID and manga ID.
pub fn get_chapter_url(chapter_id: &str, manga_id: &str) -> String {
	format!("{BASE_URL}/comics/{manga_id}/chapter/{chapter_id}")
}
