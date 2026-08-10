use std::future::Future;
use std::pin::Pin;

use thiserror::Error;

use crate::models::Video;

pub type CatalogFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<Video>, CatalogError>> + Send + 'a>>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchQuery {
    pub text: String,
    pub tags: Vec<String>,
    pub page_token: Option<String>,
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("YouTube Data API credentials are not configured")]
    NotConfigured,
    #[error("catalog request failed: {0}")]
    Request(String),
    #[error("catalog response could not be decoded: {0}")]
    InvalidResponse(String),
}

pub trait VideoCatalog: Send + Sync {
    fn default_feed(&self) -> CatalogFuture<'_>;
    fn search(&self, query: SearchQuery) -> CatalogFuture<'_>;
}
