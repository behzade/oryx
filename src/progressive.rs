use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone, Debug)]
pub struct ProgressiveDownload {
    inner: Arc<ProgressiveDownloadInner>,
}

#[derive(Debug)]
struct ProgressiveDownloadInner {
    state: Mutex<ProgressiveDownloadState>,
    wake: Condvar,
}

#[derive(Debug, Default)]
struct ProgressiveDownloadState {
    available_len: u64,
    total_len: Option<u64>,
    complete: bool,
    cancelled: bool,
    paused: bool,
    retrying: bool,
    failure: Option<String>,
}

#[derive(Debug)]
pub struct GrowingFileReader {
    file: File,
    position: u64,
    download: ProgressiveDownload,
}

#[derive(Clone, Debug)]
pub struct ProgressiveDownloadError {
    message: Arc<str>,
    cancelled: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProgressiveSnapshot {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub complete: bool,
    pub paused: bool,
}

impl ProgressiveDownload {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ProgressiveDownloadInner {
                state: Mutex::new(ProgressiveDownloadState::default()),
                wake: Condvar::new(),
            }),
        }
    }

    pub fn open_reader(&self, path: &Path) -> io::Result<GrowingFileReader> {
        let file = File::open(path)?;
        Ok(GrowingFileReader {
            file,
            position: 0,
            download: self.clone(),
        })
    }

    pub fn report_progress(&self, available_len: u64) {
        let mut state = self.inner.state.lock().expect("progressive state poisoned");
        if state.cancelled {
            return;
        }
        state.retrying = false;
        if available_len > state.available_len {
            state.available_len = available_len;
        }
        self.inner.wake.notify_all();
    }

    pub fn set_total_bytes(&self, total_len: Option<u64>) {
        let mut state = self.inner.state.lock().expect("progressive state poisoned");
        if state.cancelled {
            return;
        }
        state.retrying = false;
        state.total_len = total_len;
        self.inner.wake.notify_all();
    }

    pub fn finish(&self, available_len: u64) {
        let mut state = self.inner.state.lock().expect("progressive state poisoned");
        if state.cancelled {
            return;
        }
        state.paused = false;
        state.retrying = false;
        if available_len > state.available_len {
            state.available_len = available_len;
        }
        state.complete = true;
        self.inner.wake.notify_all();
    }

    pub fn cancel(&self) {
        let mut state = self.inner.state.lock().expect("progressive state poisoned");
        state.cancelled = true;
        state.paused = false;
        state.retrying = false;
        self.inner.wake.notify_all();
    }

    pub fn fail(&self, message: impl Into<String>) {
        let mut state = self.inner.state.lock().expect("progressive state poisoned");
        if state.cancelled {
            return;
        }
        state.paused = false;
        state.retrying = false;
        state.failure = Some(message.into());
        self.inner.wake.notify_all();
    }

    pub fn pause(&self) {
        let mut state = self.inner.state.lock().expect("progressive state poisoned");
        if state.cancelled || state.complete || state.failure.is_some() {
            return;
        }
        state.paused = true;
        self.inner.wake.notify_all();
    }

    pub fn resume(&self) {
        let mut state = self.inner.state.lock().expect("progressive state poisoned");
        if state.cancelled || state.complete || state.failure.is_some() {
            return;
        }
        state.paused = false;
        state.retrying = false;
        self.inner.wake.notify_all();
    }

    pub fn set_retrying(&self, retrying: bool) {
        let mut state = self.inner.state.lock().expect("progressive state poisoned");
        if state.cancelled || state.complete || state.failure.is_some() {
            return;
        }
        if state.paused {
            return;
        }
        state.retrying = retrying;
        self.inner.wake.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        let state = self.inner.state.lock().expect("progressive state poisoned");
        state.cancelled
    }

    pub fn is_paused(&self) -> bool {
        let state = self.inner.state.lock().expect("progressive state poisoned");
        state.paused
    }

    pub fn failure_message(&self) -> Option<String> {
        let state = self.inner.state.lock().expect("progressive state poisoned");
        state.failure.clone()
    }

    pub fn is_retrying(&self) -> bool {
        let state = self.inner.state.lock().expect("progressive state poisoned");
        state.retrying
    }

    pub fn wait_for_buffer(&self, min_bytes: u64) -> Result<(), ProgressiveDownloadError> {
        let mut state = self.inner.state.lock().expect("progressive state poisoned");
        while state.available_len < min_bytes && !state.complete {
            if state.cancelled {
                return Err(ProgressiveDownloadError::cancelled());
            }
            if let Some(message) = state.failure.clone() {
                return Err(ProgressiveDownloadError::new(message));
            }
            state = self
                .inner
                .wake
                .wait(state)
                .expect("progressive state poisoned");
        }

        if state.cancelled {
            return Err(ProgressiveDownloadError::cancelled());
        }
        if let Some(message) = state.failure.clone() {
            return Err(ProgressiveDownloadError::new(message));
        }

        Ok(())
    }

    pub fn wait_for_completion(&self) -> Result<(), ProgressiveDownloadError> {
        let mut state = self.inner.state.lock().expect("progressive state poisoned");
        while !state.complete {
            if state.cancelled {
                return Err(ProgressiveDownloadError::cancelled());
            }
            if let Some(message) = state.failure.clone() {
                return Err(ProgressiveDownloadError::new(message));
            }
            state = self
                .inner
                .wake
                .wait(state)
                .expect("progressive state poisoned");
        }

        if state.cancelled {
            return Err(ProgressiveDownloadError::cancelled());
        }
        if let Some(message) = state.failure.clone() {
            return Err(ProgressiveDownloadError::new(message));
        }

        Ok(())
    }

    pub fn snapshot(&self) -> ProgressiveSnapshot {
        let state = self.inner.state.lock().expect("progressive state poisoned");
        ProgressiveSnapshot {
            downloaded_bytes: state.available_len,
            total_bytes: state.total_len,
            complete: state.complete,
            paused: state.paused,
        }
    }

    pub fn wait_if_paused(&self) -> Result<(), ProgressiveDownloadError> {
        let mut state = self.inner.state.lock().expect("progressive state poisoned");
        while state.paused {
            if state.cancelled {
                return Err(ProgressiveDownloadError::cancelled());
            }
            if let Some(message) = state.failure.clone() {
                return Err(ProgressiveDownloadError::new(message));
            }
            if state.complete {
                return Ok(());
            }
            state = self
                .inner
                .wake
                .wait(state)
                .expect("progressive state poisoned");
        }

        if state.cancelled {
            return Err(ProgressiveDownloadError::cancelled());
        }
        if let Some(message) = state.failure.clone() {
            return Err(ProgressiveDownloadError::new(message));
        }

        Ok(())
    }

    fn availability(&self) -> io::Result<Availability> {
        let state = self.inner.state.lock().expect("progressive state poisoned");
        if state.cancelled {
            return Err(io::Error::other("Download cancelled."));
        }
        if let Some(message) = state.failure.clone() {
            return Err(io::Error::other(message));
        }

        Ok(Availability {
            available_len: state.available_len,
            total_len: state.total_len,
            complete: state.complete,
        })
    }

    fn wait_until_available(&self, target: u64) -> io::Result<Availability> {
        let mut state = self.inner.state.lock().expect("progressive state poisoned");
        while state.available_len < target && !state.complete {
            if state.cancelled {
                return Err(io::Error::other("Download cancelled."));
            }
            if let Some(message) = state.failure.clone() {
                return Err(io::Error::other(message));
            }
            if state.paused {
                state = self
                    .inner
                    .wake
                    .wait(state)
                    .expect("progressive state poisoned");
                continue;
            }
            state = self
                .inner
                .wake
                .wait(state)
                .expect("progressive state poisoned");
        }

        if state.cancelled {
            return Err(io::Error::other("Download cancelled."));
        }
        if let Some(message) = state.failure.clone() {
            return Err(io::Error::other(message));
        }

        Ok(Availability {
            available_len: state.available_len,
            total_len: state.total_len,
            complete: state.complete,
        })
    }
}

impl GrowingFileReader {
    fn read_available(&mut self, buf: &mut [u8], available_len: u64) -> io::Result<usize> {
        let remaining = available_len.saturating_sub(self.position);
        let max_read = remaining.min(buf.len() as u64) as usize;
        let bytes_read = self.file.read(&mut buf[..max_read])?;
        self.position += bytes_read as u64;
        Ok(bytes_read)
    }
}

impl Read for GrowingFileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        loop {
            let target = self.position.saturating_add(1);
            let availability = self.download.wait_until_available(target)?;

            if self.position >= availability.available_len {
                if availability.complete {
                    return Ok(0);
                }
                continue;
            }

            self.file.seek(SeekFrom::Start(self.position))?;
            let bytes_read = self.read_available(buf, availability.available_len)?;
            if bytes_read > 0 {
                return Ok(bytes_read);
            }

            if availability.complete {
                return Ok(0);
            }
        }
    }
}

impl Seek for GrowingFileReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let available = self.download.availability()?;
        let end = available.total_len.unwrap_or(available.available_len) as i128;
        let current = self.position as i128;
        let next = match pos {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::Current(offset) => current + i128::from(offset),
            SeekFrom::End(offset) => end + i128::from(offset),
        };

        if next < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek before start of file",
            ));
        }

        self.position = next as u64;
        self.file.seek(SeekFrom::Start(self.position))?;
        Ok(self.position)
    }
}

impl ProgressiveDownloadError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: Arc::<str>::from(message.into()),
            cancelled: false,
        }
    }

    fn cancelled() -> Self {
        Self {
            message: Arc::<str>::from("Download cancelled."),
            cancelled: true,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

impl fmt::Display for ProgressiveDownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProgressiveDownloadError {}

#[derive(Clone, Copy, Debug)]
struct Availability {
    available_len: u64,
    total_len: Option<u64>,
    complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn seek_from_end_uses_total_length_when_download_is_partial() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "oryx-progressive-test-{}-{unique}.bin",
            std::process::id()
        ));

        let mut file = File::create(&path).expect("temp file should be creatable");
        file.write_all(b"0123456789")
            .expect("temp file should be writable");
        drop(file);

        let download = ProgressiveDownload::new();
        download.set_total_bytes(Some(10));
        download.report_progress(4);

        let mut reader = download
            .open_reader(&path)
            .expect("reader should open the temp file");
        let position = reader
            .seek(SeekFrom::End(-2))
            .expect("seek from end should succeed");

        assert_eq!(position, 8);

        fs::remove_file(&path).expect("temp file should be removable");
    }

    #[test]
    fn pause_and_resume_are_reflected_in_snapshot() {
        let download = ProgressiveDownload::new();

        download.pause();
        assert!(download.is_paused());
        assert!(download.snapshot().paused);

        download.resume();
        assert!(!download.is_paused());
        assert!(!download.snapshot().paused);
    }
}
