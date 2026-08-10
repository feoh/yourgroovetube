use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::{Client, StatusCode};
use serde::Deserialize;

use crate::models::Video;
use crate::provider::{CatalogError, CatalogFuture, CatalogPage, SearchQuery, VideoCatalog};

const API_BASE_URL: &str = "https://www.googleapis.com/youtube/v3/";
const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

pub struct YoutubeCatalog {
    client: Client,
    api_key: String,
    region_code: String,
    results_per_page: u8,
    base_url: String,
    cache: Mutex<HashMap<CacheKey, CachedPage>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum CacheKey {
    Default(Option<String>),
    Search(SearchQuery),
}

#[derive(Clone)]
struct CachedPage {
    stored_at: Instant,
    page: CatalogPage,
}

impl YoutubeCatalog {
    pub fn new(
        api_key: impl Into<String>,
        region_code: impl Into<String>,
        results_per_page: u8,
    ) -> Result<Self, CatalogError> {
        Self::configured(api_key, region_code, results_per_page, API_BASE_URL)
    }

    fn configured(
        api_key: impl Into<String>,
        region_code: impl Into<String>,
        results_per_page: u8,
        base_url: impl Into<String>,
    ) -> Result<Self, CatalogError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(CatalogError::NotConfigured);
        }
        let region_code = region_code.into().trim().to_ascii_uppercase();
        if region_code.len() != 2 || !region_code.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            return Err(CatalogError::InvalidConfiguration(
                "region_code must be a two-letter ISO 3166-1 country code".to_owned(),
            ));
        }
        if !(1..=50).contains(&results_per_page) {
            return Err(CatalogError::InvalidConfiguration(
                "results_per_page must be between 1 and 50".to_owned(),
            ));
        }

        let client = Client::builder()
            .user_agent(concat!("yourgroovetube/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| CatalogError::Request(error.without_url().to_string()))?;
        Ok(Self {
            client,
            api_key,
            region_code,
            results_per_page,
            base_url: base_url.into(),
            cache: Mutex::new(HashMap::new()),
        })
    }

    #[cfg(test)]
    fn with_base_url(
        api_key: impl Into<String>,
        region_code: impl Into<String>,
        results_per_page: u8,
        base_url: impl Into<String>,
    ) -> Result<Self, CatalogError> {
        Self::configured(api_key, region_code, results_per_page, base_url)
    }

    async fn default_feed_inner(
        &self,
        page_token: Option<String>,
    ) -> Result<CatalogPage, CatalogError> {
        let key = CacheKey::Default(page_token.clone());
        if let Some(page) = self.cached(&key)? {
            return Ok(page);
        }

        let mut params = vec![
            ("part", "snippet,contentDetails,status".to_owned()),
            ("chart", "mostPopular".to_owned()),
            ("regionCode", self.region_code.clone()),
            ("maxResults", self.results_per_page.to_string()),
        ];
        if let Some(token) = page_token {
            params.push(("pageToken", token));
        }
        let response: VideoListResponse = self.get_json("videos", params).await?;
        let page = response.into_page();
        self.store(key, page.clone())?;
        Ok(page)
    }

    async fn search_inner(&self, query: SearchQuery) -> Result<CatalogPage, CatalogError> {
        let api_query = query.api_query();
        if api_query.is_empty() {
            return Err(CatalogError::InvalidConfiguration(
                "search text or tags must not be empty".to_owned(),
            ));
        }
        let key = CacheKey::Search(query.clone());
        if let Some(page) = self.cached(&key)? {
            return Ok(page);
        }

        let mut params = vec![
            ("part", "snippet".to_owned()),
            ("type", "video".to_owned()),
            ("q", api_query),
            ("maxResults", self.results_per_page.to_string()),
        ];
        if let Some(token) = query.page_token {
            params.push(("pageToken", token));
        }
        let response: SearchListResponse = self.get_json("search", params).await?;
        let ordered_ids = response
            .items
            .into_iter()
            .filter_map(|item| item.id.video_id)
            .collect::<Vec<_>>();
        let videos = self.hydrate(&ordered_ids).await?;
        let page = CatalogPage {
            videos,
            next_page_token: response.next_page_token,
        };
        self.store(key, page.clone())?;
        Ok(page)
    }

    async fn hydrate(&self, ordered_ids: &[String]) -> Result<Vec<Video>, CatalogError> {
        if ordered_ids.is_empty() {
            return Ok(Vec::new());
        }
        let response: VideoListResponse = self
            .get_json(
                "videos",
                vec![
                    ("part", "snippet,contentDetails,status".to_owned()),
                    ("id", ordered_ids.join(",")),
                    ("maxResults", ordered_ids.len().to_string()),
                ],
            )
            .await?;
        let mut by_id = response
            .items
            .into_iter()
            .filter_map(ApiVideo::into_video)
            .map(|video| (video.id.clone(), video))
            .collect::<HashMap<_, _>>();
        Ok(ordered_ids
            .iter()
            .filter_map(|id| by_id.remove(id))
            .collect())
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(
        &self,
        resource: &str,
        mut params: Vec<(&str, String)>,
    ) -> Result<T, CatalogError> {
        params.push(("key", self.api_key.clone()));
        let response = self
            .client
            .get(format!("{}{resource}", self.base_url))
            .query(&params)
            .send()
            .await
            .map_err(|error| CatalogError::Request(error.without_url().to_string()))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| CatalogError::Request(error.without_url().to_string()))?;
        if !status.is_success() {
            return Err(api_error(status, &body));
        }
        serde_json::from_slice(&body)
            .map_err(|error| CatalogError::InvalidResponse(error.to_string()))
    }

    fn cached(&self, key: &CacheKey) -> Result<Option<CatalogPage>, CatalogError> {
        let mut cache = self.cache.lock().map_err(|_| CatalogError::Cache)?;
        cache.retain(|_, value| value.stored_at.elapsed() < CACHE_TTL);
        Ok(cache.get(key).map(|value| value.page.clone()))
    }

    fn store(&self, key: CacheKey, page: CatalogPage) -> Result<(), CatalogError> {
        self.cache.lock().map_err(|_| CatalogError::Cache)?.insert(
            key,
            CachedPage {
                stored_at: Instant::now(),
                page,
            },
        );
        Ok(())
    }
}

impl VideoCatalog for YoutubeCatalog {
    fn default_feed(&self, page_token: Option<String>) -> CatalogFuture<'_> {
        Box::pin(self.default_feed_inner(page_token))
    }

    fn search(&self, query: SearchQuery) -> CatalogFuture<'_> {
        Box::pin(self.search_inner(query))
    }
}

fn api_error(status: StatusCode, body: &[u8]) -> CatalogError {
    let message = serde_json::from_slice::<ApiErrorEnvelope>(body)
        .map(|response| response.error.message)
        .unwrap_or_else(|_| "request failed without a readable error message".to_owned());
    CatalogError::Api {
        status: status.as_u16(),
        message,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchListResponse {
    next_page_token: Option<String>,
    #[serde(default)]
    items: Vec<SearchItem>,
}

#[derive(Deserialize)]
struct SearchItem {
    id: SearchItemId,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchItemId {
    video_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoListResponse {
    next_page_token: Option<String>,
    #[serde(default)]
    items: Vec<ApiVideo>,
}

impl VideoListResponse {
    fn into_page(self) -> CatalogPage {
        CatalogPage {
            videos: self
                .items
                .into_iter()
                .filter_map(ApiVideo::into_video)
                .collect(),
            next_page_token: self.next_page_token,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiVideo {
    id: String,
    snippet: VideoSnippet,
    content_details: Option<ContentDetails>,
    status: Option<VideoStatus>,
}

impl ApiVideo {
    fn into_video(self) -> Option<Video> {
        if self
            .status
            .as_ref()
            .and_then(|status| status.privacy_status.as_deref())
            == Some("private")
        {
            return None;
        }
        Some(Video {
            id: self.id,
            title: html_escape::decode_html_entities(&self.snippet.title).into_owned(),
            channel_title: html_escape::decode_html_entities(&self.snippet.channel_title)
                .into_owned(),
            description: html_escape::decode_html_entities(&self.snippet.description).into_owned(),
            duration_seconds: self
                .content_details
                .and_then(|details| parse_iso8601_duration(&details.duration)),
            published_at: self.snippet.published_at,
            thumbnail_url: best_thumbnail(&self.snippet.thumbnails),
            embeddable: self.status.and_then(|status| status.embeddable),
            tags: self.snippet.tags,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoSnippet {
    title: String,
    channel_title: String,
    #[serde(default)]
    description: String,
    published_at: Option<String>,
    #[serde(default)]
    thumbnails: HashMap<String, Thumbnail>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Deserialize)]
struct ContentDetails {
    duration: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoStatus {
    privacy_status: Option<String>,
    embeddable: Option<bool>,
}

#[derive(Deserialize)]
struct Thumbnail {
    url: String,
}

#[derive(Deserialize)]
struct ApiErrorEnvelope {
    error: ApiErrorBody,
}

#[derive(Deserialize)]
struct ApiErrorBody {
    message: String,
}

fn best_thumbnail(thumbnails: &HashMap<String, Thumbnail>) -> Option<String> {
    ["maxres", "standard", "high", "medium", "default"]
        .into_iter()
        .find_map(|quality| thumbnails.get(quality).map(|image| image.url.clone()))
}

fn parse_iso8601_duration(value: &str) -> Option<u64> {
    let mut characters = value.chars();
    if characters.next()? != 'P' {
        return None;
    }
    let mut number = String::new();
    let mut in_time = false;
    let mut saw_component = false;
    let mut seconds = 0.0;
    for character in characters {
        match character {
            'T' if number.is_empty() => in_time = true,
            '0'..='9' | '.' => number.push(character),
            designator => {
                let amount = number.parse::<f64>().ok()?;
                number.clear();
                saw_component = true;
                seconds += match designator {
                    'D' if !in_time => amount * 86_400.0,
                    'H' if in_time => amount * 3_600.0,
                    'M' if in_time => amount * 60.0,
                    'S' if in_time => amount,
                    _ => return None,
                };
            }
        }
    }
    if !saw_component
        || !number.is_empty()
        || !seconds.is_finite()
        || seconds < 0.0
        || seconds > u64::MAX as f64
    {
        return None;
    }
    Some(seconds.round() as u64)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn parses_youtube_iso8601_durations() {
        assert_eq!(parse_iso8601_duration("PT4M13S"), Some(253));
        assert_eq!(parse_iso8601_duration("PT1H2M3S"), Some(3_723));
        assert_eq!(parse_iso8601_duration("P1DT2H"), Some(93_600));
        assert_eq!(parse_iso8601_duration("PT"), None);
        assert_eq!(parse_iso8601_duration("not-a-duration"), None);
    }

    #[test]
    fn converts_api_video_and_selects_best_thumbnail() {
        let payload = serde_json::json!({
            "id": "abc123",
            "snippet": {
                "title": "Music &amp; Coding",
                "channelTitle": "Example Channel",
                "description": "A test video",
                "publishedAt": "2026-08-10T00:00:00Z",
                "tags": ["music", "coding"],
                "thumbnails": {
                    "default": {"url": "https://example.test/default.jpg"},
                    "high": {"url": "https://example.test/high.jpg"}
                }
            },
            "contentDetails": {"duration": "PT4M13S"},
            "status": {"privacyStatus": "public", "embeddable": true}
        });
        let Ok(api_video) = serde_json::from_value::<ApiVideo>(payload) else {
            panic!("fixture should decode");
        };
        let Some(video) = api_video.into_video() else {
            panic!("public video should be retained");
        };

        assert_eq!(video.title, "Music & Coding");
        assert_eq!(video.duration_seconds, Some(253));
        assert_eq!(
            video.thumbnail_url.as_deref(),
            Some("https://example.test/high.jpg")
        );
        assert_eq!(video.embeddable, Some(true));
    }

    #[tokio::test]
    async fn default_feed_uses_region_and_page_token() -> Result<(), Box<dyn Error + Send + Sync>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let responses = vec![
            r#"{
                "nextPageToken": "another-page",
                "items": [{
                    "id": "popular",
                    "snippet": {
                        "title": "Popular Video",
                        "channelTitle": "Channel",
                        "description": "",
                        "thumbnails": {}
                    },
                    "contentDetails": {"duration": "PT3M"},
                    "status": {"privacyStatus": "public", "embeddable": true}
                }]
            }"#
            .to_owned(),
        ];
        let server = tokio::spawn(serve_json_responses(listener, responses));
        let catalog =
            YoutubeCatalog::with_base_url("test-api-key", "ca", 15, format!("http://{address}/"))?;

        let page = catalog
            .default_feed(Some("current-page".to_owned()))
            .await?;
        let requests = server.await??;

        assert_eq!(
            page.videos.first().map(|video| video.id.as_str()),
            Some("popular")
        );
        assert_eq!(page.next_page_token.as_deref(), Some("another-page"));
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("GET /videos?"));
        assert!(requests[0].contains("chart=mostPopular"));
        assert!(requests[0].contains("regionCode=CA"));
        assert!(requests[0].contains("maxResults=15"));
        assert!(requests[0].contains("pageToken=current-page"));
        Ok(())
    }

    #[tokio::test]
    async fn search_hydrates_results_in_order_and_caches_the_page()
    -> Result<(), Box<dyn Error + Send + Sync>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let responses = vec![
            r#"{
                "nextPageToken": "next-page",
                "items": [
                    {"id": {"videoId": "first"}},
                    {"id": {"videoId": "second"}}
                ]
            }"#
            .to_owned(),
            r#"{
                "items": [
                    {
                        "id": "second",
                        "snippet": {
                            "title": "Second",
                            "channelTitle": "Channel",
                            "description": "",
                            "thumbnails": {}
                        },
                        "contentDetails": {"duration": "PT2M"},
                        "status": {"privacyStatus": "public", "embeddable": true}
                    },
                    {
                        "id": "first",
                        "snippet": {
                            "title": "First",
                            "channelTitle": "Channel",
                            "description": "",
                            "thumbnails": {}
                        },
                        "contentDetails": {"duration": "PT1M"},
                        "status": {"privacyStatus": "public", "embeddable": true}
                    }
                ]
            }"#
            .to_owned(),
        ];
        let server = tokio::spawn(serve_json_responses(listener, responses));
        let catalog =
            YoutubeCatalog::with_base_url("test-api-key", "US", 2, format!("http://{address}/"))?;
        let query = SearchQuery::new("synthwave");

        let page = catalog.search(query.clone()).await?;
        let cached_page = catalog.search(query).await?;
        let requests = server.await??;

        assert_eq!(
            page.videos
                .iter()
                .map(|video| video.id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert_eq!(page.next_page_token.as_deref(), Some("next-page"));
        assert_eq!(cached_page.videos, page.videos);
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /search?"));
        assert!(requests[0].contains("q=synthwave"));
        assert!(requests[1].starts_with("GET /videos?"));
        assert!(requests[1].contains("id=first%2Csecond"));
        Ok(())
    }

    async fn serve_json_responses(
        listener: TcpListener,
        responses: Vec<String>,
    ) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        let mut requests = Vec::new();
        for body in responses {
            let (mut stream, _) = listener.accept().await?;
            let mut request = Vec::new();
            loop {
                let mut chunk = [0_u8; 1_024];
                let read = stream.read(&mut chunk).await?;
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request)?;
            requests.push(request.lines().next().unwrap_or_default().to_owned());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
            stream.shutdown().await?;
        }
        Ok(requests)
    }

    #[test]
    fn api_errors_do_not_echo_response_bodies_without_google_message_shape() {
        let error = api_error(StatusCode::FORBIDDEN, b"api-key=should-not-be-echoed");

        assert_eq!(
            error.to_string(),
            "YouTube Data API returned HTTP 403: request failed without a readable error message"
        );
    }
}
