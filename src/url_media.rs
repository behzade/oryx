use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context as _, Result};
use serde::Deserialize;
use url::Url;

use crate::progressive::ProgressiveDownload;
use crate::provider::{DownloadRequest, HttpHeader, network_agent};

const DOWNLOAD_RETRY_LIMIT: usize = 4;
const DOWNLOAD_RETRY_BASE_DELAY_MS: u64 = 750;

#[derive(Clone)]
pub(crate) struct ResolvedVideoRequest {
    pub(crate) title: Option<String>,
    pub(crate) extension: Option<String>,
    pub(crate) duration_seconds: Option<u64>,
    pub(crate) download_request: DownloadRequest,
}

pub(crate) fn validate_open_url_input(url: &str) -> Result<Url> {
    let parsed = Url::parse(url).context("Enter a valid URL.")?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => anyhow::bail!("Only http:// and https:// URLs are supported."),
    }
    if parsed.host_str().is_none() {
        anyhow::bail!("Enter a complete URL with a host.");
    }
    Ok(parsed)
}

pub(crate) fn resolve_video_url(url: &str) -> Result<ResolvedVideoRequest> {
    let _ = validate_open_url_input(url)?;
    let output = Command::new(preferred_binary("yt-dlp", &["/opt/homebrew/bin/yt-dlp"]))
        .arg("--dump-single-json")
        .arg("--no-warnings")
        .arg("--no-playlist")
        .arg("--format")
        .arg("b/best")
        .arg(url)
        .output()
        .context("Failed to run yt-dlp")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !stderr.is_empty() {
            eprintln!("yt-dlp failed for '{url}': {stderr}");
        }
        anyhow::bail!("Could not resolve a playable media source from that URL.");
    }

    let dump: YtDlpDump =
        serde_json::from_slice(&output.stdout).context("Failed to parse yt-dlp response")?;
    let selected = dump.selected_download().context(
        "yt-dlp did not return a playable single-stream URL; a progressive format may not be available for this page",
    )?;
    let playback_url = selected
        .url
        .clone()
        .context("yt-dlp did not return a downloadable media URL")?;
    let duration_seconds = dump.duration_seconds();
    let extension = selected.ext.or(dump.ext.clone());
    let supports_byte_ranges = !is_hls_playlist_url(&playback_url);
    let headers = selected
        .http_headers
        .into_iter()
        .flat_map(|headers| headers.into_iter())
        .map(|(name, value)| HttpHeader::new(name, value))
        .collect::<Vec<_>>();

    Ok(ResolvedVideoRequest {
        title: dump.title,
        extension: extension.clone(),
        duration_seconds,
        download_request: DownloadRequest {
            url: playback_url,
            headers,
            mime_type: extension
                .as_deref()
                .and_then(mime_type_from_extension)
                .map(str::to_string),
            supports_byte_ranges,
        },
    })
}

pub(crate) fn next_download_destination(
    title: Option<&str>,
    extension: Option<&str>,
    source_url: &str,
    preferred_destination: Option<&Path>,
) -> Result<PathBuf> {
    let downloads_dir = dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Downloads")))
        .context("Downloads directory is not available")?;
    fs::create_dir_all(&downloads_dir).with_context(|| {
        format!(
            "Failed to create downloads directory {}",
            downloads_dir.display()
        )
    })?;

    Ok(build_download_destination(
        &downloads_dir,
        title,
        extension,
        source_url,
        preferred_destination,
    ))
}

pub(crate) fn download_video_to_path(
    request: &DownloadRequest,
    destination: &Path,
    progress: Option<&ProgressiveDownload>,
) -> Result<()> {
    let parent = destination.parent().with_context(|| {
        format!(
            "Destination {} has no parent directory",
            destination.display()
        )
    })?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create parent directory {}", parent.display()))?;

    if destination.is_file() && !request.supports_byte_ranges {
        if let Some(progress) = progress {
            progress.finish(downloaded_file_len(destination)?);
        }
        return Ok(());
    }

    let download_result = download_to_partial_path(
        request,
        destination,
        progress,
        DownloadRetryPolicy::Bounded(DOWNLOAD_RETRY_LIMIT),
    )?;

    if let Some(progress) = progress {
        progress.finish(download_result.total_bytes);
    }

    Ok(())
}

pub(crate) fn open_media_with_default_app(path: &Path) -> Result<()> {
    let mut command = default_open_command(path);
    command
        .spawn()
        .with_context(|| format!("Failed to open {} with the default app", path.display()))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn default_open_command(path: &Path) -> Command {
    let mut command = Command::new("open");
    command.arg(path);
    command
}

#[cfg(target_os = "windows")]
fn default_open_command(path: &Path) -> Command {
    let mut command = Command::new("cmd");
    command.arg("/C").arg("start").arg("").arg(path);
    command
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_open_command(path: &Path) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(path);
    command
}

pub(crate) fn fallback_title_for_url(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .host_str()
                .map(|host| format!("Open Media ({host})"))
                .or_else(|| {
                    parsed
                        .path_segments()
                        .and_then(|segments| segments.rev().find(|segment| !segment.is_empty()))
                        .map(sanitize_path_component)
                })
        })
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Open Media".to_string())
}

#[derive(Debug, Deserialize)]
struct YtDlpDump {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    http_headers: Option<HashMap<String, String>>,
    #[serde(default)]
    requested_downloads: Vec<YtDlpRequestedDownload>,
}

#[derive(Clone, Debug, Deserialize)]
struct YtDlpRequestedDownload {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    http_headers: Option<HashMap<String, String>>,
}

impl YtDlpDump {
    fn duration_seconds(&self) -> Option<u64> {
        self.duration
            .filter(|seconds| *seconds > 0.0)
            .map(|seconds| seconds.ceil() as u64)
    }

    fn selected_download(&self) -> Option<YtDlpRequestedDownload> {
        self.requested_downloads
            .iter()
            .cloned()
            .find(|download| {
                download
                    .url
                    .as_ref()
                    .is_some_and(|url| !url.trim().is_empty())
            })
            .or_else(|| {
                self.url.clone().map(|url| YtDlpRequestedDownload {
                    url: Some(url),
                    ext: self.ext.clone(),
                    http_headers: self.http_headers.clone(),
                })
            })
    }
}

fn build_download_destination(
    downloads_dir: &Path,
    title: Option<&str>,
    extension: Option<&str>,
    source_url: &str,
    preferred_destination: Option<&Path>,
) -> PathBuf {
    if let Some(preferred_destination) = preferred_destination {
        return preferred_destination.to_path_buf();
    }

    let base_name = title
        .filter(|title| !title.trim().is_empty())
        .map(sanitize_path_component)
        .unwrap_or_else(|| fallback_download_name(source_url));
    let extension = sanitize_extension(extension.or_else(|| extension_from_url(source_url)));
    let initial_destination = downloads_dir.join(format!("{base_name}.{extension}"));
    if !initial_destination.exists() {
        return initial_destination;
    }

    let mut duplicate_index = 2usize;
    loop {
        let candidate = downloads_dir.join(format!("{base_name} {duplicate_index}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
        duplicate_index += 1;
    }
}

pub(crate) fn fallback_download_name(source_url: &str) -> String {
    Url::parse(source_url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|segments| segments.rev().find(|segment| !segment.is_empty()))
                .map(sanitize_path_component)
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "oryx-video".to_string())
}

fn sanitize_extension(extension: Option<&str>) -> String {
    extension
        .map(|extension| {
            extension
                .trim()
                .trim_start_matches('.')
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .filter(|extension| !extension.is_empty())
        .unwrap_or_else(|| "mp4".to_string())
}

fn sanitize_path_component(input: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());
    for ch in input.chars() {
        let replacement = match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        };
        sanitized.push(replacement);
    }

    let sanitized = sanitized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(['.', ' '])
        .to_string();

    if sanitized.is_empty() {
        "Untitled".to_string()
    } else {
        sanitized
    }
}

fn extension_from_url(url: &str) -> Option<&str> {
    url.split('?')
        .next()
        .map(Path::new)
        .and_then(|path| path.extension())
        .and_then(OsStr::to_str)
}

fn mime_type_from_extension(extension: &str) -> Option<&'static str> {
    match extension.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => Some("video/mp4"),
        "webm" => Some("video/webm"),
        "mov" => Some("video/quicktime"),
        "mkv" => Some("video/x-matroska"),
        "m3u8" => Some("application/vnd.apple.mpegurl"),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug)]
enum DownloadRetryPolicy {
    Bounded(usize),
}

#[derive(Clone, Copy, Debug)]
struct DownloadResult {
    total_bytes: u64,
}

enum DownloadResponse {
    Stream(ureq::Response, u64),
    AlreadyComplete(u64),
}

fn download_to_partial_path(
    request: &DownloadRequest,
    destination: &Path,
    progress: Option<&ProgressiveDownload>,
    retry_policy: DownloadRetryPolicy,
) -> Result<DownloadResult> {
    let mut attempt = 0usize;

    loop {
        if let Some(progress) = progress {
            progress.wait_if_paused()?;
        }
        if progress.is_some_and(ProgressiveDownload::is_cancelled) {
            anyhow::bail!("Download cancelled.");
        }

        let attempt_result = if is_hls_playlist_url(&request.url) {
            download_hls_stream_to_partial_path(request, destination, progress)
        } else {
            let existing_len = resumable_download_len(destination, request.supports_byte_ranges)?;
            download_attempt(request, destination, progress, existing_len)
        };

        match attempt_result {
            Ok(result) => return Ok(result),
            Err(error)
                if should_retry_partial_download(&error)
                    && retry_policy.allows_retry(attempt)
                    && !progress.is_some_and(ProgressiveDownload::is_cancelled) =>
            {
                if let Some(progress) = progress {
                    progress.set_retrying(true);
                }
                std::thread::sleep(download_retry_delay(attempt));
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

fn download_attempt(
    request: &DownloadRequest,
    destination: &Path,
    progress: Option<&ProgressiveDownload>,
    existing_len: u64,
) -> Result<DownloadResult> {
    if let Some(progress) = progress {
        progress.wait_if_paused()?;
        progress.set_retrying(false);
    }
    let (response, resume_from) = match open_download_response(request, existing_len)? {
        DownloadResponse::Stream(response, resume_from) => (response, resume_from),
        DownloadResponse::AlreadyComplete(total_bytes) => {
            if let Some(progress) = progress {
                progress.set_total_bytes(Some(total_bytes));
                progress.finish(total_bytes);
            }
            return Ok(DownloadResult { total_bytes });
        }
    };
    let expected_total = expected_total_bytes(&response, resume_from);
    if let Some(progress) = progress {
        progress.set_total_bytes(expected_total);
        if resume_from > 0 {
            progress.report_progress(resume_from);
        }
    }

    let mut reader = response.into_reader();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resume_from > 0)
        .truncate(resume_from == 0)
        .open(destination)
        .with_context(|| format!("Failed to open temporary file {}", destination.display()))?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut total_bytes = resume_from;

    loop {
        if let Some(progress) = progress {
            progress.wait_if_paused()?;
        }
        if progress.is_some_and(ProgressiveDownload::is_cancelled) {
            anyhow::bail!("Download cancelled.");
        }

        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        file.write_all(&buffer[..bytes_read])?;
        total_bytes += bytes_read as u64;
        if let Some(progress) = progress {
            progress.report_progress(total_bytes);
        }
    }
    file.flush()?;

    Ok(DownloadResult { total_bytes })
}

fn download_hls_stream_to_partial_path(
    request: &DownloadRequest,
    destination: &Path,
    progress: Option<&ProgressiveDownload>,
) -> Result<DownloadResult> {
    let master_playlist = fetch_text_response(request)?;
    let media_playlist_url = resolve_hls_media_playlist_url(request, &master_playlist)?;
    let media_playlist = if media_playlist_url == request.url {
        master_playlist
    } else {
        fetch_text_response(&DownloadRequest {
            url: media_playlist_url.clone(),
            headers: request.headers.clone(),
            mime_type: None,
            supports_byte_ranges: false,
        })?
    };
    let segment_urls = parse_hls_media_segments(&media_playlist)
        .into_iter()
        .map(|segment| resolve_hls_url(&media_playlist_url, &segment))
        .collect::<Result<Vec<_>>>()?;

    if segment_urls.is_empty() {
        anyhow::bail!("HLS media playlist did not contain any segments");
    }

    if let Some(progress) = progress {
        progress.set_total_bytes(None);
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(destination)
        .with_context(|| format!("Failed to open temporary file {}", destination.display()))?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut total_bytes = 0u64;

    for segment_url in segment_urls {
        if let Some(progress) = progress {
            progress.wait_if_paused()?;
        }
        if progress.is_some_and(ProgressiveDownload::is_cancelled) {
            anyhow::bail!("Download cancelled.");
        }

        let segment_request = DownloadRequest {
            url: segment_url,
            headers: request.headers.clone(),
            mime_type: request.mime_type.clone(),
            supports_byte_ranges: false,
        };
        let response = match open_download_response(&segment_request, 0)? {
            DownloadResponse::Stream(response, _) => response,
            DownloadResponse::AlreadyComplete(_) => {
                anyhow::bail!("HLS segment request unexpectedly reported completion")
            }
        };
        let mut reader = response.into_reader();

        loop {
            let bytes_read = reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            file.write_all(&buffer[..bytes_read])?;
            total_bytes += bytes_read as u64;
            if let Some(progress) = progress {
                progress.report_progress(total_bytes);
            }
        }
    }

    file.flush()?;

    Ok(DownloadResult { total_bytes })
}

fn build_download_request(request: &DownloadRequest) -> ureq::Request {
    let mut response = network_agent().get(&request.url);
    for header in &request.headers {
        response = response.set(&header.name, &header.value);
    }
    response
}

fn fetch_text_response(request: &DownloadRequest) -> Result<String> {
    build_download_request(request)
        .call()
        .with_context(|| format!("Failed to download {}", request.url))?
        .into_string()
        .with_context(|| format!("Failed to read response body for {}", request.url))
}

fn open_download_response(
    request: &DownloadRequest,
    existing_len: u64,
) -> Result<DownloadResponse> {
    let mut response = build_download_request(request);

    if request.supports_byte_ranges && existing_len > 0 {
        response = response.set("Range", &format!("bytes={existing_len}-"));
    }

    let response = match response.call() {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response))
            if request.supports_byte_ranges && existing_len > 0 && status == 416 =>
        {
            let Some(total_bytes) = expected_total_bytes(&response, 0) else {
                anyhow::bail!(
                    "Server rejected the resume request for {} and did not report the total size",
                    request.url
                );
            };
            if existing_len >= total_bytes {
                return Ok(DownloadResponse::AlreadyComplete(total_bytes));
            }
            anyhow::bail!(
                "Server rejected the resume request for {} even though the local file is incomplete",
                request.url
            );
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to download {}", request.url));
        }
    };
    let status = response.status();

    if existing_len > 0 {
        if request.supports_byte_ranges {
            if status != 206 {
                anyhow::bail!(
                    "Server did not honor byte-range resume request for {} (status {status})",
                    request.url
                );
            }
            return Ok(DownloadResponse::Stream(response, existing_len));
        }

        return Ok(DownloadResponse::Stream(response, 0));
    }

    Ok(DownloadResponse::Stream(response, 0))
}

fn resumable_download_len(destination: &Path, supports_byte_ranges: bool) -> Result<u64> {
    if !supports_byte_ranges || !destination.is_file() {
        return Ok(0);
    }

    Ok(fs::metadata(destination)
        .with_context(|| format!("Failed to inspect temporary file {}", destination.display()))?
        .len())
}

fn expected_total_bytes(response: &ureq::Response, resume_from: u64) -> Option<u64> {
    parse_content_range_total(response.header("Content-Range")).or_else(|| {
        response
            .header("Content-Length")
            .and_then(|value| value.parse::<u64>().ok())
            .map(|len| len.saturating_add(resume_from))
    })
}

fn parse_content_range_total(value: Option<&str>) -> Option<u64> {
    value
        .and_then(|value| value.rsplit('/').next())
        .filter(|total| *total != "*")
        .and_then(|total| total.parse::<u64>().ok())
}

fn is_hls_playlist_url(url: &str) -> bool {
    url.split('?')
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase()
        .ends_with(".m3u8")
}

fn resolve_hls_media_playlist_url(
    request: &DownloadRequest,
    master_playlist: &str,
) -> Result<String> {
    let variants = parse_hls_variants(master_playlist);
    let Some(variant) = choose_hls_variant(&variants) else {
        return Ok(request.url.clone());
    };

    resolve_hls_url(&request.url, &variant.uri)
}

#[derive(Clone, Debug)]
struct HlsVariant {
    uri: String,
    bandwidth: Option<u64>,
}

fn parse_hls_variants(playlist: &str) -> Vec<HlsVariant> {
    if !playlist.contains("#EXT-X-STREAM-INF") {
        return Vec::new();
    }

    let mut variants = Vec::new();
    let mut pending_bandwidth = None;

    for raw_line in playlist.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(attributes) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending_bandwidth = parse_hls_bandwidth(attributes);
            continue;
        }

        if line.starts_with('#') {
            continue;
        }

        variants.push(HlsVariant {
            uri: line.to_string(),
            bandwidth: pending_bandwidth.take(),
        });
    }

    variants
}

fn choose_hls_variant(variants: &[HlsVariant]) -> Option<&HlsVariant> {
    variants
        .iter()
        .max_by_key(|variant| variant.bandwidth.unwrap_or(0))
}

fn parse_hls_bandwidth(attributes: &str) -> Option<u64> {
    attributes
        .split(',')
        .find_map(|attribute| attribute.trim().strip_prefix("BANDWIDTH="))
        .and_then(|value| value.parse::<u64>().ok())
}

fn parse_hls_media_segments(playlist: &str) -> Vec<String> {
    playlist
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn resolve_hls_url(base_url: &str, uri: &str) -> Result<String> {
    if uri.contains("://") {
        return Ok(uri.to_string());
    }

    let base = Url::parse(base_url).with_context(|| format!("Invalid HLS base URL {base_url}"))?;
    let resolved = base
        .join(uri)
        .with_context(|| format!("Failed to resolve HLS URI {uri} against {base_url}"))?;
    Ok(resolved.into())
}

fn should_retry_partial_download(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| is_retryable_download_error_message(cause))
}

fn is_retryable_download_error_message(message: &dyn std::fmt::Display) -> bool {
    let text = message.to_string().to_ascii_lowercase();
    [
        "timed out",
        "timeout",
        "connection reset",
        "connection aborted",
        "broken pipe",
        "unexpected eof",
        "temporarily unavailable",
        "network is unreachable",
        "connection closed before message completed",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn download_retry_delay(attempt: usize) -> Duration {
    let exponent = attempt.min(4) as u32;
    Duration::from_millis(DOWNLOAD_RETRY_BASE_DELAY_MS.saturating_mul(1u64 << exponent))
}

impl DownloadRetryPolicy {
    fn allows_retry(self, attempt: usize) -> bool {
        match self {
            Self::Bounded(limit) => attempt < limit,
        }
    }
}

fn downloaded_file_len(path: &Path) -> Result<u64> {
    Ok(fs::metadata(path)
        .with_context(|| format!("Failed to read downloaded file metadata {}", path.display()))?
        .len())
}

fn preferred_binary<'a>(fallback: &'a str, candidates: &[&'a str]) -> &'a str {
    candidates
        .iter()
        .copied()
        .find(|candidate| Path::new(candidate).is_file())
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_path_component_replaces_invalid_characters() {
        assert_eq!(sanitize_path_component("a/b:c*"), "a_b_c_");
    }

    #[test]
    fn fallback_download_name_uses_url_tail() {
        assert_eq!(
            fallback_download_name("https://example.com/path/video.mp4?x=1"),
            "video.mp4"
        );
    }

    #[test]
    fn build_download_destination_numbers_colliding_base_name() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let downloads_dir = std::env::temp_dir().join(format!(
            "oryx-open-url-test-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&downloads_dir);
        let existing = downloads_dir.join("Example Video.mp4");
        let _ = fs::write(&existing, b"partial");

        let destination = build_download_destination(
            &downloads_dir,
            Some("Example Video"),
            Some("mp4"),
            "https://example.com/video",
            None,
        );

        assert_eq!(destination, downloads_dir.join("Example Video 2.mp4"));

        let _ = fs::remove_file(&existing);
        let _ = fs::remove_dir(&downloads_dir);
    }

    #[test]
    fn build_download_destination_honors_preferred_destination_for_resume() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let downloads_dir = std::env::temp_dir().join(format!(
            "oryx-open-url-test-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&downloads_dir);
        let existing = downloads_dir.join("Example Video.mp4");
        let _ = fs::write(&existing, b"partial");

        let destination = build_download_destination(
            &downloads_dir,
            Some("Example Video"),
            Some("mp4"),
            "https://example.com/video",
            Some(&existing),
        );

        assert_eq!(destination, existing);

        let _ = fs::remove_file(&existing);
        let _ = fs::remove_dir(&downloads_dir);
    }
}
