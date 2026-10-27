//! Canned network for native tests. Each URL maps to one reply. A URL without a reply fails
//! like a refused connection, so a request the test did not expect surfaces at the call.

use crate::http::Transport;
use ito_rs::{Error, Result, net::NetResponse};
use std::{cell::RefCell, collections::HashMap};

/// One request the code under test sent.
#[derive(Debug, Clone)]
pub struct Sent {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl Sent {
    /// Returns the value of header `name`, matched exactly as the plugin spells it.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.as_str() == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Default)]
pub struct Canned {
    replies: HashMap<String, (i32, Vec<u8>)>,
    sent: RefCell<Vec<Sent>>,
}

impl Canned {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers requests for `url` with `status` and `body`.
    pub fn on(mut self, url: &str, status: i32, body: impl AsRef<[u8]>) -> Self {
        self.replies
            .insert(url.to_string(), (status, body.as_ref().to_vec()));
        self
    }

    /// Every request sent so far, in order.
    pub fn requests(&self) -> Vec<Sent> {
        self.sent.borrow().clone()
    }

    /// How many requests were sent to `url`.
    pub fn sent_to(&self, url: &str) -> usize {
        self.sent
            .borrow()
            .iter()
            .filter(|sent| sent.url == url)
            .count()
    }
}

impl Transport for Canned {
    fn send(&self, url: &str, headers: &[(&str, &str)]) -> Result<NetResponse> {
        self.sent.borrow_mut().push(Sent {
            url: url.to_string(),
            headers: headers
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
        });
        let (status, body) = self
            .replies
            .get(url)
            .ok_or_else(|| Error::Net(format!("no canned reply for {url}")))?;
        Ok(NetResponse {
            status: *status,
            headers: HashMap::new(),
            body: body.clone(),
        })
    }
}
