//! Request parameters, their canonical signed form (spec section 2.1), and the URL query that
//! carries them. Both come from one parameter model, so the signed values and sent values agree.

use std::collections::BTreeMap;

/// One request parameter. Its shape decides the bracket syntax the site reads.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Param {
    /// `name=value`.
    Text(String),
    /// `name[0]=a&name[1]=b`, in list order.
    List(Vec<String>),
    /// `name[key]=value`, with keys in ascending order.
    Map(BTreeMap<&'static str, String>),
}

/// The parameters of one request. Names are fixed site names; empty optional parameters are
/// simply not added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    params: BTreeMap<&'static str, Param>,
}

impl Query {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.params.insert(name, Param::Text(value.into()));
        self
    }

    pub fn int(self, name: &'static str, value: i32) -> Self {
        self.text(name, value.to_string())
    }

    pub fn list(mut self, name: &'static str, values: &[&str]) -> Self {
        let values = values.iter().map(|value| (*value).to_owned()).collect();
        self.params.insert(name, Param::List(values));
        self
    }

    /// `name[key]=direction`, the site's single-key sort order.
    pub fn order(mut self, key: &'static str, direction: &'static str) -> Self {
        let map = BTreeMap::from([(key, direction.to_owned())]);
        self.params.insert("order", Param::Map(map));
        self
    }

    /// The `name=value` pairs in canonical order, values as sent (not yet encoded).
    fn pairs(&self) -> Vec<(String, &str)> {
        let mut pairs = Vec::new();
        for (name, param) in &self.params {
            match param {
                Param::Text(value) => pairs.push(((*name).to_owned(), value.as_str())),
                Param::List(values) => {
                    for (index, value) in values.iter().enumerate() {
                        pairs.push((format!("{name}[{index}]"), value.as_str()));
                    }
                }
                Param::Map(map) => {
                    for (key, value) in map {
                        pairs.push((format!("{name}[{key}]"), value.as_str()));
                    }
                }
            }
        }
        pairs
    }

    /// The signed message for `path` (relative to the API base): the path, then the raw pairs.
    pub fn message(&self, path: &str) -> String {
        let pairs = self.pairs();
        let mut message = path.to_owned();
        for (index, (name, value)) in pairs.iter().enumerate() {
            message.push(if index == 0 { '?' } else { '&' });
            message.push_str(name);
            message.push('=');
            message.push_str(value);
        }
        message
    }

    /// The URL query for these parameters, names and values percent-encoded and in the signed
    /// order. The signature covers the raw form, so the server sees the same parameters either way.
    /// Empty when there are no parameters.
    pub fn encoded(&self) -> String {
        self.pairs()
            .iter()
            .map(|(name, value)| {
                format!(
                    "{}={}",
                    urlencoding::encode(name),
                    urlencoding::encode(value)
                )
            })
            .collect::<Vec<_>>()
            .join("&")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_without_parameters_signs_the_path_alone() {
        let query = Query::new();
        assert_eq!(query.message("/manga/8v88"), "/manga/8v88");
        assert_eq!(query.encoded(), "");
    }

    #[test]
    fn top_level_names_sort_and_nested_shapes_keep_their_order() {
        let query = Query::new()
            .int("page", 1)
            .list("content_rating", &["safe", "suggestive"])
            .text("keyword", "solo")
            .int("limit", 28)
            .order("chapter_updated_at", "desc");
        assert_eq!(
            query.message("/manga"),
            "/manga?content_rating[0]=safe&content_rating[1]=suggestive&keyword=solo&limit=28&order[chapter_updated_at]=desc&page=1"
        );
    }

    #[test]
    fn signed_values_are_raw_and_sent_values_are_encoded() {
        let query = Query::new().text("keyword", "solo leveling");
        assert_eq!(query.message("/manga"), "/manga?keyword=solo leveling");
        assert_eq!(query.encoded(), "keyword=solo%20leveling");
    }

    #[test]
    fn unicode_and_reserved_characters_stay_literal_in_the_message() {
        let query = Query::new()
            .text("keyword", "日本語 & a=b")
            .int("limit", 1)
            .int("page", 1);
        assert_eq!(
            query.message("/manga"),
            "/manga?keyword=日本語 & a=b&limit=1&page=1"
        );
        assert_eq!(
            query.encoded(),
            "keyword=%E6%97%A5%E6%9C%AC%E8%AA%9E%20%26%20a%3Db&limit=1&page=1"
        );
    }

    #[test]
    fn url_names_are_percent_encoded_while_the_signed_message_stays_raw() {
        let query = Query::new()
            .list("content_rating", &["safe"])
            .order("number", "desc")
            .int("page", 1);
        assert_eq!(
            query.message("/manga/8v88/chapters"),
            "/manga/8v88/chapters?content_rating[0]=safe&order[number]=desc&page=1"
        );
        assert_eq!(
            query.encoded(),
            "content_rating%5B0%5D=safe&order%5Bnumber%5D=desc&page=1"
        );
    }
}
