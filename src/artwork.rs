use std::time::Duration;

use image::{DynamicImage, ImageBuffer, Rgba};
use ratatui::{Frame, layout::Rect};
use ratatui_image::{Resize, StatefulImage, picker::Picker, protocol::StatefulProtocol};
use reqwest::{Client, StatusCode, Url, redirect::Policy};
use thiserror::Error;

const MAX_THUMBNAIL_BYTES: u64 = 10 * 1024 * 1024;
const THUMBNAIL_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ArtworkState {
    client: Client,
    picker: Picker,
    protocol: Option<StatefulProtocol>,
    loaded_url: Option<String>,
}

#[derive(Debug, Error)]
pub enum ArtworkError {
    #[error("thumbnail URL is not an approved HTTPS YouTube image URL")]
    UntrustedUrl,
    #[error("could not create the thumbnail HTTP client: {0}")]
    Client(String),
    #[error("thumbnail request failed: {0}")]
    Request(String),
    #[error("thumbnail request returned HTTP {0}")]
    Http(StatusCode),
    #[error("thumbnail is larger than the 10 MiB safety limit")]
    TooLarge,
    #[error("thumbnail is not a supported image: {0}")]
    Decode(#[from] image::ImageError),
}

impl ArtworkState {
    pub fn detect() -> Result<Self, ArtworkError> {
        let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        Self::new(picker)
    }

    pub fn halfblocks() -> Result<Self, ArtworkError> {
        Self::new(Picker::halfblocks())
    }

    fn new(picker: Picker) -> Result<Self, ArtworkError> {
        let client = Client::builder()
            .user_agent(concat!("yourgroovetube/", env!("CARGO_PKG_VERSION")))
            .redirect(Policy::limited(3))
            .timeout(THUMBNAIL_REQUEST_TIMEOUT)
            .build()
            .map_err(|error| ArtworkError::Client(error.without_url().to_string()))?;
        let mut artwork = Self {
            client,
            picker,
            protocol: None,
            loaded_url: None,
        };
        artwork.set_image(Self::placeholder());
        Ok(artwork)
    }

    pub fn protocol_name(&self) -> String {
        format!("{:?}", self.picker.protocol_type())
    }

    pub async fn load_url(&mut self, url: &str) -> Result<(), ArtworkError> {
        if self.loaded_url.as_deref() == Some(url) {
            return Ok(());
        }
        let url = trusted_thumbnail_url(url)?;
        let response = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(|error| ArtworkError::Request(error.without_url().to_string()))?;
        trusted_thumbnail_url(response.url().as_str())?;
        if !response.status().is_success() {
            return Err(ArtworkError::Http(response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_THUMBNAIL_BYTES)
        {
            return Err(ArtworkError::TooLarge);
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| ArtworkError::Request(error.without_url().to_string()))?;
        if bytes.len() as u64 > MAX_THUMBNAIL_BYTES {
            return Err(ArtworkError::TooLarge);
        }
        self.set_image(image::load_from_memory(&bytes)?);
        self.loaded_url = Some(url.to_string());
        Ok(())
    }

    pub fn show_placeholder(&mut self) {
        if self.loaded_url.take().is_some() {
            self.set_image(Self::placeholder());
        }
    }

    pub fn render(&mut self, frame: &mut Frame<'_>, area: Rect) {
        if let Some(protocol) = self.protocol.as_mut() {
            frame.render_stateful_widget(
                StatefulImage::default().resize(Resize::Fit(None)),
                area,
                protocol,
            );
        }
    }

    fn set_image(&mut self, image: DynamicImage) {
        self.protocol = Some(self.picker.new_resize_protocol(image));
    }

    fn placeholder() -> DynamicImage {
        let image = ImageBuffer::from_fn(320, 180, |x, y| {
            let horizontal = ((x * 255) / 319) as u8;
            let vertical = ((y * 180) / 179) as u8;
            let pulse = ((x.abs_diff(160) + y.abs_diff(90)) / 4).min(90) as u8;
            Rgba([
                horizontal.saturating_sub(pulse / 2),
                180_u8.saturating_sub(pulse),
                vertical.saturating_add(50),
                255,
            ])
        });
        DynamicImage::ImageRgba8(image)
    }
}

fn trusted_thumbnail_url(value: &str) -> Result<Url, ArtworkError> {
    let url = Url::parse(value).map_err(|_| ArtworkError::UntrustedUrl)?;
    let trusted_host = url.host_str().is_some_and(|host| {
        let host = host.to_ascii_lowercase();
        host == "ytimg.com" || host.ends_with(".ytimg.com")
    });
    if url.scheme() != "https"
        || !trusted_host
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ArtworkError::UntrustedUrl);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_youtube_thumbnail_hosts_are_trusted() {
        assert!(trusted_thumbnail_url("https://i.ytimg.com/vi/abc/hqdefault.jpg").is_ok());
        assert!(trusted_thumbnail_url("https://ytimg.com/vi/abc/default.jpg").is_ok());
        assert!(trusted_thumbnail_url("http://i.ytimg.com/vi/abc/default.jpg").is_err());
        assert!(trusted_thumbnail_url("https://ytimg.com.example.test/image.jpg").is_err());
        assert!(trusted_thumbnail_url("https://user@i.ytimg.com/image.jpg").is_err());
    }

    #[tokio::test]
    #[ignore = "requires network access to a YouTube thumbnail"]
    async fn downloads_and_decodes_a_real_youtube_thumbnail() -> Result<(), ArtworkError> {
        let mut artwork = ArtworkState::halfblocks()?;

        artwork
            .load_url("https://i.ytimg.com/vi/jNQXAC9IVRw/hqdefault.jpg")
            .await?;

        assert!(artwork.loaded_url.is_some());
        assert!(artwork.protocol.is_some());
        Ok(())
    }

    #[test]
    fn placeholder_uses_video_aspect_ratio() {
        let placeholder = ArtworkState::placeholder();

        assert_eq!(placeholder.width(), 320);
        assert_eq!(placeholder.height(), 180);
    }
}
