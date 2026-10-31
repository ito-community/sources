//! Where the source's pages and data come from.
//!
//! The source reads the site through one small interface, so everything built on it can be run
//! against recorded pages in native tests; the web view is only reachable on a host.

use crate::web::{self, Reply};
use ito_rs::Result;
use scraper::Html;

pub trait Site {
    /// The page at `path` of the site, after any verification it asked of the user.
    fn page(path: &str) -> Result<Html>;

    /// The site's answer to a data request for `path`, made from inside one of its pages.
    fn reply(path: &str) -> Result<Reply>;
}

/// The site as Ito's web view sees it.
pub struct Web;

impl Site for Web {
    fn page(path: &str) -> Result<Html> {
        web::load(path)
    }

    fn reply(path: &str) -> Result<Reply> {
        web::fetch(path)
    }
}
