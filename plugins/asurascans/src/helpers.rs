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

/// Recognises a Cloudflare verification page from its markers only; the body is never returned.
fn is_challenge_page(body: &[u8]) -> bool {
	let text = String::from_utf8_lossy(body);
	text.contains("cf-chl-")
		|| text.contains("<title>Just a moment")
		|| text.contains("<title>Attention Required!")
}

/// Maps a page that did not load to its error. A verification page means the site is asking the
/// user, so the host opens the site for it.
pub fn check_response(status: i32, body: &[u8]) -> ito_rs::Result<()> {
	if is_challenge_page(body) {
		return Err(ito_rs::Error::NeedsLogin(
			"Open Asura Scans in the source browser to complete site verification".into(),
		));
	}
	match status {
		200..=299 => Ok(()),
		401 => Err(ito_rs::Error::NeedsLogin(
			"Sign in to Asura Scans in the source browser".into(),
		)),
		403 => Err(ito_rs::Error::Server("Asura Scans denied the request".into())),
		404 => Err(ito_rs::Error::NotFound(
			"Asura Scans could not find that page".into(),
		)),
		status => Err(ito_rs::Error::Server(format!(
			"Asura Scans returned HTTP {status}"
		))),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn verification_pages_are_reported_as_needing_the_site() {
		let challenge = b"<html><head><title>Just a moment...</title></head></html>";
		assert!(matches!(
			check_response(403, challenge),
			Err(ito_rs::Error::NeedsLogin(_))
		));
		assert!(matches!(
			check_response(200, challenge),
			Err(ito_rs::Error::NeedsLogin(_))
		));
		assert!(matches!(
			check_response(503, b"<script src='/cf-chl-x.js'></script>"),
			Err(ito_rs::Error::NeedsLogin(_))
		));
	}

	#[test]
	fn refusals_and_missing_pages_keep_their_categories() {
		assert!(check_response(200, b"<h1>Series</h1>").is_ok());
		assert!(matches!(
			check_response(403, b"<html>denied</html>"),
			Err(ito_rs::Error::Server(_))
		));
		assert!(matches!(
			check_response(404, b""),
			Err(ito_rs::Error::NotFound(_))
		));
		assert!(matches!(
			check_response(500, b""),
			Err(ito_rs::Error::Server(_))
		));
	}
}
