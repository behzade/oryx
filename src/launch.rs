use std::env;
use std::ffi::OsString;
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::io::{BufRead, BufReader};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
#[cfg(unix)]
use std::time::Duration;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::url_media::validate_open_url_input;

#[cfg(unix)]
const IPC_READ_TIMEOUT: Duration = Duration::from_secs(2);
const NATIVE_MESSAGING_MAX_MESSAGE_LEN: usize = 64 * 1024 * 1024;

pub(crate) struct LaunchOptions {
    pub(crate) initial_open_url: Option<String>,
    pub(crate) ipc_rx: Option<Receiver<String>>,
}

pub(crate) fn prepare() -> Result<Option<LaunchOptions>> {
    if is_native_messaging_launch(env::args_os().skip(1)) {
        return handle_native_messaging().map(|()| None);
    }

    let initial_open_url = parse_launch_args(env::args_os().skip(1))?;
    if let Some(url) = initial_open_url.as_deref() {
        if send_open_url_to_running_instance(url)? {
            return Ok(None);
        }
    }

    Ok(Some(LaunchOptions {
        initial_open_url,
        ipc_rx: start_open_url_ipc_listener(),
    }))
}

fn is_native_messaging_launch<I>(args: I) -> bool
where
    I: IntoIterator<Item = OsString>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    matches!(args.as_slice(), [flag] if flag == "--native-messaging")
}

fn parse_launch_args<I>(args: I) -> Result<Option<String>>
where
    I: IntoIterator<Item = OsString>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(None),
        [flag, raw_url] if flag == "--open-url" => normalize_external_url(raw_url),
        [raw] => {
            let raw = raw.to_string_lossy();
            if raw.starts_with("oryx://") {
                parse_deep_link(&raw).map(Some)
            } else {
                anyhow::bail!(usage_message());
            }
        }
        _ => anyhow::bail!(usage_message()),
    }
}

fn normalize_external_url(raw_url: &OsString) -> Result<Option<String>> {
    let url = raw_url
        .to_str()
        .context("--open-url requires a valid UTF-8 URL")?;
    Ok(Some(validate_open_url_input(url)?.to_string()))
}

fn parse_deep_link(raw: &str) -> Result<String> {
    let parsed = Url::parse(raw).context("Enter a valid oryx:// URL.")?;
    if parsed.scheme() != "oryx" || parsed.host_str() != Some("open") {
        anyhow::bail!("Only oryx://open?url=<encoded-url> links are supported.");
    }

    let url = parsed
        .query_pairs()
        .find_map(|(name, value)| (name == "url").then(|| value.into_owned()))
        .context("oryx://open links require a url query parameter.")?;
    Ok(validate_open_url_input(&url)?.to_string())
}

fn usage_message() -> &'static str {
    "Usage: oryx [--open-url <http-url>|oryx://open?url=<encoded-http-url>]"
}

fn handle_native_messaging() -> Result<()> {
    handle_native_messaging_io(std::io::stdin().lock(), std::io::stdout().lock())
}

fn handle_native_messaging_io<R, W>(mut reader: R, mut writer: W) -> Result<()>
where
    R: Read,
    W: Write,
{
    let response = match read_native_message(&mut reader).and_then(handle_native_messaging_request)
    {
        Ok(()) => NativeMessagingResponse {
            ok: true,
            error: None,
        },
        Err(error) => NativeMessagingResponse {
            ok: false,
            error: Some(format!("{error:#}")),
        },
    };
    write_native_message(&mut writer, &response)
}

fn handle_native_messaging_request(request: NativeMessagingRequest) -> Result<()> {
    if request
        .action
        .as_deref()
        .is_some_and(|action| action != "open_url")
    {
        anyhow::bail!("Unsupported native messaging action.");
    }

    let url = validate_open_url_input(&request.url)?.to_string();
    if send_open_url_to_running_instance(&url)? {
        return Ok(());
    }

    spawn_open_url_instance(&url)
}

fn read_native_message<R>(reader: &mut R) -> Result<NativeMessagingRequest>
where
    R: Read,
{
    let mut len_bytes = [0; 4];
    reader
        .read_exact(&mut len_bytes)
        .context("Failed to read native messaging message length")?;
    let len = u32::from_ne_bytes(len_bytes) as usize;
    if len > NATIVE_MESSAGING_MAX_MESSAGE_LEN {
        anyhow::bail!("Native messaging message is too large.");
    }

    let mut message = vec![0; len];
    reader
        .read_exact(&mut message)
        .context("Failed to read native messaging message body")?;
    serde_json::from_slice(&message).context("Failed to parse native messaging request")
}

fn write_native_message<W, T>(writer: &mut W, message: &T) -> Result<()>
where
    W: Write,
    T: Serialize,
{
    let payload =
        serde_json::to_vec(message).context("Failed to serialize native messaging response")?;
    let len = u32::try_from(payload.len()).context("Native messaging response is too large")?;
    writer
        .write_all(&len.to_ne_bytes())
        .context("Failed to write native messaging response length")?;
    writer
        .write_all(&payload)
        .context("Failed to write native messaging response body")?;
    writer
        .flush()
        .context("Failed to flush native messaging response")
}

fn spawn_open_url_instance(url: &str) -> Result<()> {
    let current_exe = env::current_exe().context("Failed to resolve Oryx executable path")?;
    Command::new(current_exe)
        .arg("--open-url")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to start Oryx")?;
    Ok(())
}

#[derive(Deserialize)]
struct NativeMessagingRequest {
    action: Option<String>,
    url: String,
}

#[derive(Serialize)]
struct NativeMessagingResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[cfg(unix)]
fn send_open_url_to_running_instance(url: &str) -> Result<bool> {
    let socket_path = ipc_socket_path();
    if !socket_path.exists() {
        return Ok(false);
    }

    match UnixStream::connect(&socket_path) {
        Ok(mut stream) => {
            stream
                .write_all(url.as_bytes())
                .context("Failed to send URL to running Oryx instance")?;
            stream
                .write_all(b"\n")
                .context("Failed to finish URL IPC message")?;
            Ok(true)
        }
        Err(_) => {
            let _ = fs::remove_file(&socket_path);
            Ok(false)
        }
    }
}

#[cfg(not(unix))]
fn send_open_url_to_running_instance(_url: &str) -> Result<bool> {
    Ok(false)
}

#[cfg(unix)]
fn start_open_url_ipc_listener() -> Option<Receiver<String>> {
    let socket_path = ipc_socket_path();
    prepare_socket_parent(&socket_path).ok()?;

    if socket_path.exists() {
        if UnixStream::connect(&socket_path).is_ok() {
            return None;
        }
        let _ = fs::remove_file(&socket_path);
    }

    let listener = UnixListener::bind(&socket_path).ok()?;
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("open-url-ipc-listener".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let _ = stream.set_read_timeout(Some(IPC_READ_TIMEOUT));
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let url = line.trim().to_string();
                let Ok(url) = validate_open_url_input(&url).map(|url| url.to_string()) else {
                    continue;
                };
                if tx.send(url).is_err() {
                    break;
                }
            }
            let _ = fs::remove_file(&socket_path);
        })
        .ok()?;

    Some(rx)
}

#[cfg(not(unix))]
fn start_open_url_ipc_listener() -> Option<Receiver<String>> {
    None
}

#[cfg(unix)]
fn prepare_socket_parent(socket_path: &Path) -> Result<()> {
    let parent = socket_path
        .parent()
        .context("Oryx IPC socket path has no parent")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create IPC directory {}", parent.display()))
}

#[cfg(unix)]
fn ipc_socket_path() -> PathBuf {
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    ipc_socket_path_for_runtime_dir(runtime_dir)
}

#[cfg(unix)]
fn ipc_socket_path_for_runtime_dir(runtime_dir: PathBuf) -> PathBuf {
    runtime_dir.join("oryx").join("open-url.sock")
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    #[cfg(unix)]
    use super::ipc_socket_path_for_runtime_dir;
    use super::parse_launch_args;

    #[test]
    fn parses_open_url_flag() {
        let parsed = parse_launch_args([
            OsString::from("--open-url"),
            OsString::from("https://example.com/watch?v=1"),
        ])
        .expect("open-url flag should parse");

        assert_eq!(parsed.as_deref(), Some("https://example.com/watch?v=1"));
    }

    #[test]
    fn parses_open_deep_link() {
        let parsed = parse_launch_args([OsString::from(
            "oryx://open?url=https%3A%2F%2Fexample.com%2Fmedia.mp4%3Fx%3D1",
        )])
        .expect("deep link should parse");

        assert_eq!(parsed.as_deref(), Some("https://example.com/media.mp4?x=1"));
    }

    #[test]
    fn rejects_non_media_deep_link_url() {
        let error = parse_launch_args([OsString::from("oryx://open?url=file%3A%2F%2Ftmp%2Fa.mp4")])
            .expect_err("file URLs should be rejected");

        assert!(error.to_string().contains("Only http:// and https://"));
    }

    #[test]
    fn rejects_unknown_arguments() {
        let error = parse_launch_args([OsString::from("--help")])
            .expect_err("unknown arguments should be rejected");

        assert!(error.to_string().contains("Usage: oryx"));
    }

    #[test]
    #[cfg(unix)]
    fn ipc_socket_path_uses_runtime_dir() {
        assert_eq!(
            ipc_socket_path_for_runtime_dir("/tmp/oryx-runtime-test".into()),
            std::path::Path::new("/tmp/oryx-runtime-test")
                .join("oryx")
                .join("open-url.sock")
        );
    }
}
