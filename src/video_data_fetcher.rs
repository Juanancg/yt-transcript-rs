use regex::Regex;
use reqwest::Client;
use serde_json::json;

use crate::captions_extractor::CaptionsExtractor;
use crate::errors::{CouldNotRetrieveTranscript, CouldNotRetrieveTranscriptReason};
use crate::js_var_parser::JsVarParser;
use crate::microformat_extractor::MicroformatExtractor;
use crate::models::{MicroformatData, StreamingData, VideoDetails, VideoInfos};
use crate::playability_asserter::PlayabilityAsserter;
use crate::streaming_data_extractor::StreamingDataExtractor;
use crate::transcript_list::TranscriptList;
use crate::video_details_extractor::VideoDetailsExtractor;
use crate::youtube_page_fetcher::YoutubePageFetcher;

const INNERTUBE_API_URL: &str = "https://www.youtube.com/youtubei/v1/player?key={api_key}";
const INNERTUBE_CLIENT_NAME: &str = "ANDROID";
const INNERTUBE_CLIENT_VERSION: &str = "20.10.38";

/// # VideoDataFetcher
///
/// Core component responsible for fetching transcript data and video details from YouTube.
///
/// This struct handles the low-level communication with YouTube's web API to:
/// - Fetch available transcripts for a video
/// - Extract caption JSON data from YouTube pages
/// - Retrieve detailed information about videos, including metadata
///
/// The VideoDataFetcher works by parsing YouTube's HTML and JavaScript variables
/// to extract the necessary data, since YouTube doesn't provide a public API for transcripts.
///
/// ## Internal Architecture
///
/// This component uses several helper classes to process data:
/// - `YoutubePageFetcher`: Handles HTTP requests to YouTube, including proxy support
/// - `JsVarParser`: Extracts JavaScript variables from YouTube's HTML
/// - `PlayabilityAsserter`: Verifies video availability and access permissions
/// - `VideoDetailsExtractor`: Extracts detailed information from video data
pub struct VideoDataFetcher {
    /// HTTP client for making requests
    pub client: Client,
    /// Specialized fetcher for YouTube pages
    page_fetcher: YoutubePageFetcher,
}

impl VideoDataFetcher {
    /// Creates a new VideoDataFetcher instance.
    ///
    /// # Parameters
    ///
    /// * `client` - A configured reqwest HTTP client to use for requests
    /// * `proxy_config` - Optional proxy configuration for routing requests through a proxy
    ///
    /// # Returns
    ///
    /// A new VideoDataFetcher instance.
    ///
    /// # Example (internal usage)
    ///
    /// ```rust,no_run
    /// # use reqwest::Client;
    /// # use yt_transcript_rs::video_data_fetcher::VideoDataFetcher;
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Create a client
    /// let client = Client::new();
    /// // Create the fetcher
    /// let fetcher = VideoDataFetcher::new(
    ///     client
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(client: Client) -> Self {
        let page_fetcher = YoutubePageFetcher::new(client.clone());

        Self {
            client,
            page_fetcher,
        }
    }

    /// Fetches the list of available transcripts for a YouTube video.
    ///
    /// This method:
    /// 1. Retrieves the video page HTML
    /// 2. Extracts the captions JSON data
    /// 3. Builds a TranscriptList from the extracted data
    ///
    /// # Parameters
    ///
    /// * `video_id` - The YouTube video ID (e.g., "dQw4w9WgXcQ")
    ///
    /// # Returns
    ///
    /// * `Result<TranscriptList, CouldNotRetrieveTranscript>` - A TranscriptList on success, or an error if retrieval fails
    ///
    /// # Errors
    ///
    /// This method can fail if:
    /// - The video doesn't exist or is private
    /// - The video has no available transcripts
    /// - YouTube's HTML structure has changed and parsing fails
    /// - Network errors occur during the request
    ///
    /// # Example (internal usage)
    ///
    /// ```rust,no_run
    /// # use yt_transcript_rs::api::YouTubeTranscriptApi;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let api = YouTubeTranscriptApi::new(None, None, None)?;
    /// let video_id = "dQw4w9WgXcQ";
    ///
    /// // This internally calls VideoDataFetcher::fetch_transcript_list
    /// let transcript_list = api.list_transcripts(video_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn fetch_transcript_list(
        &self,
        video_id: &str,
    ) -> Result<TranscriptList, CouldNotRetrieveTranscript> {
        // Mirror python implementation: fetch captions through InnerTube player API
        // using the API key extracted from the watch page.
        let video_captions = self.fetch_captions_from_innertube(video_id).await?;

        TranscriptList::build(video_id.to_string(), &video_captions)
    }

    /// Fetches detailed information about a YouTube video.
    ///
    /// This method retrieves comprehensive metadata about a video, including:
    /// - Title, author, channel ID
    /// - View count and video length
    /// - Thumbnails in various resolutions
    /// - Keywords and description
    ///
    /// # Parameters
    ///
    /// * `video_id` - The YouTube video ID
    ///
    /// # Returns
    ///
    /// * `Result<VideoDetails, CouldNotRetrieveTranscript>` - Video details on success, or an error
    ///
    /// # Errors
    ///
    /// Similar to transcript fetching, this can fail if:
    /// - The video doesn't exist or is private
    /// - YouTube's HTML structure has changed and parsing fails
    /// - Network errors occur during the request
    ///
    /// # Example (internal usage)
    ///
    /// ```rust,no_run
    /// # use yt_transcript_rs::api::YouTubeTranscriptApi;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let api = YouTubeTranscriptApi::new(None, None, None)?;
    /// let video_id = "dQw4w9WgXcQ";
    ///
    /// // This internally calls VideoDataFetcher::fetch_video_details
    /// let details = api.fetch_video_details(video_id).await?;
    ///
    /// println!("Video title: {}", details.title);
    /// println!("Author: {}", details.author);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn fetch_video_details(
        &self,
        video_id: &str,
    ) -> Result<VideoDetails, CouldNotRetrieveTranscript> {
        // Get player response with playability check
        let player_response = self.fetch_player_response(video_id, true).await?;

        // Extract video details from player response
        VideoDetailsExtractor::extract_video_details(&player_response, video_id)
    }

    /// Fetches microformat data for a YouTube video.
    ///
    /// This method retrieves additional metadata about a video, including:
    /// - Available countries
    /// - Category
    /// - Embed information
    /// - Information about whether the video is unlisted, family-safe, etc.
    ///
    /// # Parameters
    ///
    /// * `video_id` - The YouTube video ID
    ///
    /// # Returns
    ///
    /// * `Result<MicroformatData, CouldNotRetrieveTranscript>` - Microformat data on success, or an error
    ///
    /// # Errors
    ///
    /// This method can fail if:
    /// - The video doesn't exist or is private
    /// - YouTube's HTML structure has changed and parsing fails
    /// - Network errors occur during the request
    ///
    /// # Example (internal usage)
    ///
    /// ```rust,no_run
    /// # use yt_transcript_rs::api::YouTubeTranscriptApi;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let api = YouTubeTranscriptApi::new(None, None, None)?;
    /// let video_id = "dQw4w9WgXcQ";
    ///
    /// // This internally calls VideoDataFetcher::fetch_microformat
    /// let microformat = api.fetch_microformat(video_id).await?;
    ///
    /// if let Some(category) = &microformat.category {
    ///     println!("Video category: {}", category);
    /// }
    ///
    /// if let Some(countries) = &microformat.available_countries {
    ///     println!("Available in {} countries", countries.len());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn fetch_microformat(
        &self,
        video_id: &str,
    ) -> Result<MicroformatData, CouldNotRetrieveTranscript> {
        // Get player response with playability check
        let player_response = self.fetch_player_response(video_id, true).await?;

        // Extract microformat data from player response
        MicroformatExtractor::extract_microformat_data(&player_response, video_id)
    }

    /// Fetches streaming data for a YouTube video.
    ///
    /// This method retrieves information about available video and audio formats, including:
    /// - URLs for different quality versions of the video
    /// - Resolution, bitrate, and codec information
    /// - Both combined formats (with audio and video) and separate adaptive formats
    /// - Information about format expiration
    ///
    /// # Parameters
    ///
    /// * `video_id` - The YouTube video ID
    ///
    /// # Returns
    ///
    /// * `Result<StreamingData, CouldNotRetrieveTranscript>` - Streaming data on success, or an error
    ///
    /// # Errors
    ///
    /// This method can fail if:
    /// - The video doesn't exist or is private
    /// - The video has geo-restrictions that prevent access
    /// - YouTube's HTML structure has changed and parsing fails
    /// - Network errors occur during the request
    ///
    /// # Example (internal usage)
    ///
    /// ```rust,no_run
    /// # use yt_transcript_rs::api::YouTubeTranscriptApi;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let api = YouTubeTranscriptApi::new(None, None, None)?;
    /// let video_id = "dQw4w9WgXcQ";
    ///
    /// // This internally calls VideoDataFetcher::fetch_streaming_data
    /// let streaming = api.fetch_streaming_data(video_id).await?;
    ///
    /// // Print information about available formats
    /// println!("Available formats: {}", streaming.formats.len());
    /// println!("Adaptive formats: {}", streaming.adaptive_formats.len());
    /// println!("Expires in: {} seconds", streaming.expires_in_seconds);
    ///
    /// // Find highest quality video format
    /// if let Some(best_format) = streaming.adaptive_formats.iter()
    ///     .filter(|f| f.width.is_some() && f.height.is_some())
    ///     .max_by_key(|f| f.height.unwrap_or(0)) {
    ///     println!("Highest quality: {}p", best_format.height.unwrap());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn fetch_streaming_data(
        &self,
        video_id: &str,
    ) -> Result<StreamingData, CouldNotRetrieveTranscript> {
        // Get player response with playability check
        let player_response = self.fetch_player_response(video_id, true).await?;

        // Extract streaming data from player response
        StreamingDataExtractor::extract_streaming_data(&player_response, video_id)
    }

    /// Fetches all available information about a YouTube video in a single request.
    ///
    /// This method retrieves the video page once and extracts all data, including:
    /// - Video details (title, author, etc.)
    /// - Microformat data (category, available countries, etc.)
    /// - Streaming data (available formats, qualities, etc.)
    /// - Transcript list (available caption languages)
    ///
    /// This is more efficient than calling the individual fetch methods separately
    /// when multiple types of information are needed, as it avoids multiple HTTP requests.
    ///
    /// # Parameters
    ///
    /// * `video_id` - The YouTube video ID
    ///
    /// # Returns
    ///
    /// * `Result<VideoInfos, CouldNotRetrieveTranscript>` - Combined video information on success, or an error
    ///
    /// # Errors
    ///
    /// This method can fail if:
    /// - The video doesn't exist or is private
    /// - YouTube's HTML structure has changed and parsing fails
    /// - Network errors occur during the request
    ///
    /// # Example (internal usage)
    ///
    /// ```rust,no_run
    /// # use yt_transcript_rs::api::YouTubeTranscriptApi;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let api = YouTubeTranscriptApi::new(None, None, None)?;
    /// let video_id = "dQw4w9WgXcQ";
    ///
    /// // This internally calls VideoDataFetcher::fetch_video_infos
    /// let infos = api.fetch_video_infos(video_id).await?;
    ///
    /// println!("Title: {}", infos.video_details.title);
    /// println!("Category: {}", infos.microformat.category.unwrap_or_default());
    /// println!("Available transcripts: {}", infos.transcript_list.transcripts().count());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn fetch_video_infos(
        &self,
        video_id: &str,
    ) -> Result<VideoInfos, CouldNotRetrieveTranscript> {
        // Get player response with playability check (single network request)
        let player_response = self.fetch_player_response(video_id, true).await?;

        // Extract all data in parallel using the various extractors
        let video_details =
            VideoDetailsExtractor::extract_video_details(&player_response, video_id)?;
        let microformat =
            MicroformatExtractor::extract_microformat_data(&player_response, video_id)?;
        let streaming_data =
            StreamingDataExtractor::extract_streaming_data(&player_response, video_id)?;

        // Extract captions data and build transcript list
        let captions_data = CaptionsExtractor::extract_captions_data(&player_response, video_id)?;
        let transcript_list = TranscriptList::build(video_id.to_string(), &captions_data)?;

        // Combine all data into the VideoInfos struct
        Ok(VideoInfos {
            video_details,
            microformat,
            streaming_data,
            transcript_list,
        })
    }

    /// Extracts the InnerTube API key from watch page HTML.
    ///
    /// Python implementation parity:
    /// this key is read from the watch page and then used to call
    /// `youtubei/v1/player?key=...` with the Android client context.
    fn extract_innertube_api_key(
        &self,
        html: &str,
        video_id: &str,
    ) -> Result<String, CouldNotRetrieveTranscript> {
        let pattern = Regex::new(r#"\"INNERTUBE_API_KEY\":\s*\"([a-zA-Z0-9_-]+)\""#)
            .expect("valid API key regex");

        if let Some(captures) = pattern.captures(html) {
            if let Some(api_key) = captures.get(1) {
                return Ok(api_key.as_str().to_string());
            }
        }

        if html.contains("class=\"g-recaptcha\"") {
            return Err(CouldNotRetrieveTranscript {
                video_id: video_id.to_string(),
                reason: Some(CouldNotRetrieveTranscriptReason::IpBlocked(None)),
            });
        }

        Err(CouldNotRetrieveTranscript {
            video_id: video_id.to_string(),
            reason: Some(CouldNotRetrieveTranscriptReason::YouTubeDataUnparsable(
                "Could not extract INNERTUBE_API_KEY from watch page HTML".to_string(),
            )),
        })
    }

    async fn fetch_innertube_player_response(
        &self,
        video_id: &str,
        api_key: &str,
    ) -> Result<serde_json::Value, CouldNotRetrieveTranscript> {
        let response = self
            .client
            .post(INNERTUBE_API_URL.replace("{api_key}", api_key))
            .json(&json!({
                "context": {
                    "client": {
                        "clientName": INNERTUBE_CLIENT_NAME,
                        "clientVersion": INNERTUBE_CLIENT_VERSION,
                    }
                },
                "videoId": video_id,
            }))
            .send()
            .await
            .map_err(|e| CouldNotRetrieveTranscript {
                video_id: video_id.to_string(),
                reason: Some(CouldNotRetrieveTranscriptReason::YouTubeRequestFailed(
                    e.to_string(),
                )),
            })?;

        if !response.status().is_success() {
            return Err(CouldNotRetrieveTranscript {
                video_id: video_id.to_string(),
                reason: Some(CouldNotRetrieveTranscriptReason::YouTubeRequestFailed(
                    format!("YouTube returned status code: {}", response.status()),
                )),
            });
        }

        response
            .json::<serde_json::Value>()
            .await
            .map_err(|e| CouldNotRetrieveTranscript {
                video_id: video_id.to_string(),
                reason: Some(CouldNotRetrieveTranscriptReason::YouTubeDataUnparsable(
                    format!("Failed to parse InnerTube player response: {}", e),
                )),
            })
    }

    async fn fetch_captions_from_innertube(
        &self,
        video_id: &str,
    ) -> Result<serde_json::Value, CouldNotRetrieveTranscript> {
        let html = self.page_fetcher.fetch_video_page(video_id).await?;
        let api_key = self.extract_innertube_api_key(&html, video_id)?;
        let player_response = self
            .fetch_innertube_player_response(video_id, &api_key)
            .await?;

        PlayabilityAsserter::assert_playability(&player_response, video_id)?;
        CaptionsExtractor::extract_captions_data(&player_response, video_id)
    }

    /// Extracts the ytInitialPlayerResponse JavaScript variable from YouTube's HTML.
    ///
    /// This variable contains detailed information about the video, including captions.
    ///
    /// # Parameters
    ///
    /// * `html` - The HTML content of the YouTube video page
    /// * `video_id` - The YouTube video ID (used for error reporting)
    ///
    /// # Returns
    ///
    /// * `Result<serde_json::Value, CouldNotRetrieveTranscript>` - The parsed JavaScript object or an error
    fn extract_yt_initial_player_response(
        &self,
        html: &str,
        video_id: &str,
    ) -> Result<serde_json::Value, CouldNotRetrieveTranscript> {
        let js_var_parser = JsVarParser::new("ytInitialPlayerResponse");
        let player_response = js_var_parser.parse(html, video_id)?;

        Ok(player_response)
    }

    /// Helper method that fetches a video page and extracts the player response.
    ///
    /// This private method centralizes the common functionality used across multiple
    /// data fetching methods, eliminating code duplication.
    ///
    /// # Parameters
    ///
    /// * `video_id` - The YouTube video ID
    /// * `check_playability` - Whether to verify the video is playable
    ///
    /// # Returns
    ///
    /// * `Result<serde_json::Value, CouldNotRetrieveTranscript>` - The player response JSON or an error
    async fn fetch_player_response(
        &self,
        video_id: &str,
        check_playability: bool,
    ) -> Result<serde_json::Value, CouldNotRetrieveTranscript> {
        // Fetch the video page HTML only once
        let html = self.page_fetcher.fetch_video_page(video_id).await?;

        // Extract the player response
        let player_response = self.extract_yt_initial_player_response(&html, video_id)?;

        // Check playability status if requested
        if check_playability {
            PlayabilityAsserter::assert_playability(&player_response, video_id)?;
        }

        Ok(player_response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_innertube_api_key_from_html() {
        let fetcher = VideoDataFetcher::new(Client::new());
        let html =
            r#"<html><script>var cfg = {"INNERTUBE_API_KEY":"test_api_key_123"};</script></html>"#;

        let api_key = fetcher
            .extract_innertube_api_key(html, "video123")
            .expect("expected to extract API key");

        assert_eq!(api_key, "test_api_key_123");
    }

    #[test]
    fn extract_innertube_api_key_returns_ip_blocked_for_recaptcha() {
        let fetcher = VideoDataFetcher::new(Client::new());
        let html = r#"<html><div class="g-recaptcha"></div></html>"#;

        let err = fetcher
            .extract_innertube_api_key(html, "video123")
            .expect_err("expected API key extraction to fail");

        assert!(matches!(
            err.reason,
            Some(CouldNotRetrieveTranscriptReason::IpBlocked(_))
        ));
    }
}
