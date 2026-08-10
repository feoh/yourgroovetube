use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

use crate::models::Video;

pub type CatalogFuture<'a> =
    Pin<Box<dyn Future<Output = Result<CatalogPage, CatalogError>> + Send + 'a>>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CatalogPage {
    pub videos: Vec<Video>,
    pub next_page_token: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct SearchQuery {
    pub text: String,
    pub tags: Vec<String>,
    pub page_token: Option<String>,
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn api_query(&self) -> String {
        std::iter::once(self.text.as_str())
            .chain(self.tags.iter().map(String::as_str))
            .map(str::trim)
            .filter(|term| !term.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("YouTube Data API credentials are not configured")]
    NotConfigured,
    #[error("invalid YouTube catalog configuration: {0}")]
    InvalidConfiguration(String),
    #[error("catalog request failed: {0}")]
    Request(String),
    #[error("YouTube Data API returned HTTP {status}: {message}")]
    Api { status: u16, message: String },
    #[error("catalog response could not be decoded: {0}")]
    InvalidResponse(String),
    #[error("catalog cache is unavailable")]
    Cache,
}

pub trait VideoCatalog: Send + Sync {
    fn default_feed(&self, page_token: Option<String>) -> CatalogFuture<'_>;
    fn search(&self, query: SearchQuery) -> CatalogFuture<'_>;
    fn playlist<'a>(
        &'a self,
        playlist_id: &'a str,
        page_token: Option<String>,
    ) -> CatalogFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_query_combines_title_text_and_tags() {
        let query = SearchQuery {
            text: "synthwave mix".to_owned(),
            tags: vec!["#retrowave".to_owned(), "  #instrumental ".to_owned()],
            page_token: None,
        };

        assert_eq!(query.api_query(), "synthwave mix #retrowave #instrumental");
    }
}
