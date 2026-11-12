//! nhentai search text. Gallery lookup and the permanent tag exclusion live here so
//! they can be tested without the host.

use std::collections::HashSet;

/// Sexual-content tags for minors. Always excluded. Not a setting.
pub const BLOCKED_TAGS: &[&str] = &["lolicon", "shotacon"];

/// Ids for [`BLOCKED_TAGS`] from the public tag endpoint. Gallery detail still
/// checks names, so a changed id cannot reopen those galleries.
pub const BLOCKED_TAG_IDS: &[i64] = &[19_440, 32_341];

const META_LANGUAGES: &[&str] = &["translated", "rewrite", "speechless"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Date,
    PopularToday,
    PopularWeek,
    PopularMonth,
    Popular,
}

impl Sort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Date => "date",
            Self::PopularToday => "popular-today",
            Self::PopularWeek => "popular-week",
            Self::PopularMonth => "popular-month",
            Self::Popular => "popular",
        }
    }

    pub fn from_control(value: Option<&str>) -> Self {
        match value {
            Some("popular-today") => Self::PopularToday,
            Some("popular-week") => Self::PopularWeek,
            Some("popular-month") => Self::PopularMonth,
            Some("popular") => Self::Popular,
            _ => Self::Date,
        }
    }

    pub fn from_listing(id: &str) -> Option<Self> {
        match id {
            "latest" => Some(Self::Date),
            "popular-today" => Some(Self::PopularToday),
            "popular-week" => Some(Self::PopularWeek),
            "popular-month" => Some(Self::PopularMonth),
            "popular" => Some(Self::Popular),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchParts {
    pub text: String,
    pub languages: Vec<String>,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub excluded_tags: Vec<String>,
    pub artists: Vec<String>,
    pub characters: Vec<String>,
    pub parodies: Vec<String>,
    pub groups: Vec<String>,
    pub pages_min: Option<i64>,
    pub pages_max: Option<i64>,
    pub extra: String,
}

impl SearchParts {
    pub fn browse(languages: &[String], blocked: &[String], extra: &str) -> Self {
        Self {
            text: String::new(),
            languages: languages.to_vec(),
            categories: Vec::new(),
            tags: Vec::new(),
            excluded_tags: blocked.to_vec(),
            artists: Vec::new(),
            characters: Vec::new(),
            parodies: Vec::new(),
            groups: Vec::new(),
            pages_min: None,
            pages_max: None,
            extra: extra.to_owned(),
        }
    }

    /// A query the search endpoint will accept. Permanent exclusions are last so
    /// they are not cancelled by anything the user typed.
    pub fn build(&self) -> String {
        let mut clauses = Vec::new();
        let mut seen = HashSet::new();
        let text = clean_free_text(&self.text);
        if !text.is_empty() {
            clauses.push(text);
        }
        push_terms(&mut clauses, &mut seen, "language", &self.languages, false);
        push_terms(&mut clauses, &mut seen, "category", &self.categories, false);
        push_terms(&mut clauses, &mut seen, "tag", &self.tags, false);
        push_terms(&mut clauses, &mut seen, "artist", &self.artists, false);
        push_terms(
            &mut clauses,
            &mut seen,
            "character",
            &self.characters,
            false,
        );
        push_terms(&mut clauses, &mut seen, "parody", &self.parodies, false);
        push_terms(&mut clauses, &mut seen, "group", &self.groups, false);
        clauses.extend(page_clauses(self.pages_min, self.pages_max));
        push_terms(&mut clauses, &mut seen, "tag", &self.excluded_tags, true);
        let extra = clean_free_text(&self.extra);
        if !extra.is_empty() {
            clauses.push(extra);
        }
        for tag in BLOCKED_TAGS {
            clauses.push(clause("tag", tag, true));
        }
        clauses.join(" ")
    }
}

pub fn is_blocked_name(name: &str) -> bool {
    let name = name.trim();
    BLOCKED_TAGS
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case(name))
}

pub fn is_blocked_id(id: i64) -> bool {
    BLOCKED_TAG_IDS.contains(&id)
}

pub fn mentions_blocked(input: &str) -> bool {
    tokenize(input).iter().any(|token| blocked_inclusion(token))
}

pub fn is_meta_language(name: &str) -> bool {
    META_LANGUAGES
        .iter()
        .any(|tag| tag.eq_ignore_ascii_case(name.trim()))
}

/// A positive gallery id pasted as a number or an nhentai gallery URL.
pub fn lookup_id(query: &str) -> Option<i64> {
    let query = query.trim();
    if let Ok(id) = query.parse::<i64>() {
        return (id > 0).then_some(id);
    }
    gallery_id_from_url(query)
}

pub fn gallery_id_from_url(url: &str) -> Option<i64> {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/')?;
    let host = host.split('@').next_back()?.split(':').next()?;
    if !matches!(host, "nhentai.net" | "www.nhentai.net") {
        return None;
    }
    let path = path.split(['?', '#']).next()?;
    let rest = path.strip_prefix("g/")?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let after = &rest[digits.len()..];
    if !after.is_empty() && !after.starts_with('/') {
        return None;
    }
    digits.parse().ok().filter(|id| *id > 0)
}

pub fn suggestion_prefix(query: &str) -> Option<(&'static str, &'static str, &str)> {
    const PREFIXES: &[(&str, &str, &str)] = &[
        ("artist:", "artist", "artist"),
        ("character:", "character", "character"),
        ("parody:", "parody", "parody"),
        ("group:", "group", "group"),
        ("language:", "language", "language"),
        ("category:", "category", "category"),
        ("tag:", "tag", "tag"),
    ];
    let query = query.trim();
    for (prefix, kind, clause_prefix) in PREFIXES {
        if let Some(rest) = query.strip_prefix(prefix) {
            let rest = rest.trim().trim_matches('"');
            return (!rest.is_empty()).then_some((*kind, *clause_prefix, rest));
        }
    }
    (!query.is_empty()).then_some(("tag", "tag", query))
}

pub fn quoted_clause(prefix: &str, name: &str) -> String {
    clause(prefix, name, false)
}

fn page_clauses(min: Option<i64>, max: Option<i64>) -> Vec<String> {
    let mut clauses = Vec::new();
    if let Some(min) = min.filter(|n| *n > 1) {
        clauses.push(format!("pages:>{}", min - 1));
    }
    if let Some(max) = max {
        clauses.push(format!("pages:<{}", max.saturating_add(1)));
    }
    clauses
}

fn push_terms(
    out: &mut Vec<String>,
    seen: &mut HashSet<String>,
    prefix: &str,
    names: &[String],
    exclude: bool,
) {
    for name in names {
        let name = name.trim();
        if name.is_empty()
            || is_blocked_name(name)
            || (is_meta_language(name) && prefix == "language")
        {
            continue;
        }
        let key = format!(
            "{}{prefix}:{}",
            if exclude { "-" } else { "" },
            name.to_lowercase()
        );
        if seen.insert(key) {
            out.push(clause(prefix, name, exclude));
        }
    }
}

fn clause(prefix: &str, name: &str, exclude: bool) -> String {
    let sign = if exclude { "-" } else { "" };
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    format!("{sign}{prefix}:\"{escaped}\"")
}

fn clean_free_text(input: &str) -> String {
    tokenize(input)
        .into_iter()
        .filter(|token| !blocked_inclusion(token))
        .collect::<Vec<_>>()
        .join(" ")
}

fn blocked_inclusion(token: &str) -> bool {
    let token = token.trim().trim_start_matches('+');
    if is_blocked_name(token) {
        return true;
    }
    let Some(rest) = token
        .strip_prefix("tag:")
        .or_else(|| token.strip_prefix("tags:"))
    else {
        return false;
    };
    is_blocked_name(&unquote(rest))
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[0] == *bytes.last().unwrap_or(&0)
    {
        return value[1..value.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\'", "'");
    }
    value.to_owned()
}

fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escape = false;
    for ch in input.chars() {
        if quote.is_some() {
            current.push(ch);
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
                continue;
            }
            if Some(ch) == quote {
                quote = None;
            }
            continue;
        }
        if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
        }
        current.push(ch);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts() -> SearchParts {
        SearchParts::browse(&[], &[], "")
    }

    #[test]
    fn browse_query_always_excludes_minor_tags_last() {
        let query = parts().build();
        let tokens = tokenize(&query);
        assert!(tokens.iter().any(|token| token == "-tag:\"lolicon\""));
        assert!(tokens.iter().any(|token| token == "-tag:\"shotacon\""));
        assert!(!tokens.iter().any(|token| token == "tag:\"lolicon\""));
        assert!(query.ends_with("-tag:\"lolicon\" -tag:\"shotacon\""));
    }

    #[test]
    fn user_text_cannot_include_a_blocked_tag() {
        let mut parts = parts();
        parts.text = "school tag:\"lolicon\" tag:vanilla +tag:shotacon".into();
        parts.extra = "tag:lolicon pages:>10 -tag:\"shotacon\"".into();
        parts.tags = vec!["lolicon".into(), "vanilla".into()];
        parts.excluded_tags = vec!["yaoi".into()];
        let tokens = tokenize(&parts.build());
        assert!(tokens.iter().any(|token| token == "school"));
        assert!(tokens.iter().any(|token| token == "tag:\"vanilla\""));
        assert!(tokens.iter().any(|token| token == "pages:>10"));
        assert!(tokens.iter().any(|token| token == "-tag:\"yaoi\""));
        let blocked: Vec<&str> = tokens
            .iter()
            .filter(|token| token.contains("lolicon") || token.contains("shotacon"))
            .map(String::as_str)
            .collect();
        assert_eq!(
            blocked,
            ["-tag:\"shotacon\"", "-tag:\"lolicon\"", "-tag:\"shotacon\""]
        );
    }

    #[test]
    fn page_bounds_are_inclusive_and_languages_are_quoted() {
        let mut parts = parts();
        parts.languages = vec!["english".into(), "english".into(), "translated".into()];
        parts.pages_min = Some(20);
        parts.pages_max = Some(30);
        parts.artists = vec!["a name".into()];
        let query = parts.build();
        assert!(query.contains("language:\"english\""));
        assert_eq!(query.matches("language:\"english\"").count(), 1);
        assert!(!query.contains("translated"));
        assert!(query.contains("pages:>19"));
        assert!(query.contains("pages:<31"));
        assert!(query.contains("artist:\"a name\""));
    }

    #[test]
    fn gallery_urls_and_ids_parse_only_for_this_site() {
        assert_eq!(lookup_id("696294"), Some(696294));
        assert_eq!(lookup_id(" 0 "), None);
        assert_eq!(
            gallery_id_from_url("https://nhentai.net/g/696294/3?page=1"),
            Some(696294)
        );
        assert_eq!(
            gallery_id_from_url("http://www.nhentai.net/g/12/"),
            Some(12)
        );
        assert_eq!(gallery_id_from_url("https://nhentai.net/search/?q=1"), None);
        assert_eq!(gallery_id_from_url("https://evil.example/g/12"), None);
        assert_eq!(gallery_id_from_url("https://nhentai.net/g/12abc"), None);
        assert_eq!(lookup_id("https://nhentai.net/g/9"), Some(9));
    }

    #[test]
    fn suggestion_prefix_selects_the_tag_type() {
        assert_eq!(
            suggestion_prefix("artist:foo"),
            Some(("artist", "artist", "foo"))
        );
        assert_eq!(suggestion_prefix("school"), Some(("tag", "tag", "school")));
        assert_eq!(suggestion_prefix("tag:"), None);
    }
}
