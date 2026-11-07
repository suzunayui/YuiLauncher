//! Streams a release asset to a file and hashes what was written.

use crate::releases::agent;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub enum DownloadError {
    Cancelled,
    Failed(String),
}

pub type Progress<'a> = &'a mut dyn FnMut(u64, Option<u64>);

pub trait Downloader: Send + Sync {
    /// Returns the SHA-256 of what was written, in lower-case hex.
    fn download(&self, url: &str, dest: &Path, expected_size: Option<u64>, cancel: &AtomicBool, on_progress: Progress) -> Result<String, DownloadError>;
}

pub struct HttpDownloader;

impl Downloader for HttpDownloader {
    fn download(&self, url: &str, dest: &Path, expected_size: Option<u64>, cancel: &AtomicBool, on_progress: Progress) -> Result<String, DownloadError> {
        let failed = |error: &dyn std::fmt::Display| DownloadError::Failed(format!("ダウンロードに失敗しました（{error}）"));
        if cancel.load(Ordering::SeqCst) {
            return Err(DownloadError::Cancelled);
        }
        let agent = agent(Duration::from_secs(30)).map_err(|error| failed(&error))?;
        let response = agent.get(url).set("User-Agent", "YuiLauncher").call().map_err(|error| match error {
            ureq::Error::Status(code, _) => failed(&code),
            ureq::Error::Transport(transport) => failed(&transport.kind()),
        })?;
        let mut reader = response.into_reader();
        let mut file = std::fs::File::create(dest).map_err(|error| failed(&error))?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0u8; 64 * 1024];
        let mut received = 0u64;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(DownloadError::Cancelled);
            }
            let read = reader.read(&mut buffer).map_err(|error| failed(&error))?;
            if read == 0 {
                break;
            }
            hash.update(&buffer[..read]);
            file.write_all(&buffer[..read]).map_err(|error| failed(&error))?;
            received += read as u64;
            on_progress(received, expected_size);
        }
        file.flush().map_err(|error| failed(&error))?;
        if expected_size.is_some_and(|size| size != received) {
            return Err(DownloadError::Failed("ダウンロードしたファイルのサイズが一致しません".to_string()));
        }
        let mut hex = String::with_capacity(64);
        for byte in hash.finalize() {
            let _ = write!(hex, "{byte:02x}");
        }
        Ok(hex)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{serve, temp_dir};

    const SIZE: usize = 256 * 1024;

    fn response(body: &[u8]) -> Vec<u8> {
        let mut bytes = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    fn body() -> Vec<u8> {
        (0..SIZE).map(|index| (index % 251) as u8).collect()
    }

    #[test]
    fn writes_the_file_reports_progress_and_returns_its_hash() {
        let body = body();
        let (root, _) = serve(vec![response(&body)]);
        let dir = temp_dir();
        let dest = dir.join("file.bin");
        let mut last = 0;
        let sha = HttpDownloader
            .download(&root, &dest, Some(SIZE as u64), &AtomicBool::new(false), &mut |received, _| last = received)
            .unwrap();
        let mut expected = String::new();
        for byte in Sha256::digest(&body) {
            write!(expected, "{byte:02x}").unwrap();
        }
        assert_eq!(sha, expected);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(last, SIZE as u64);
    }

    #[test]
    fn fails_when_the_size_differs_from_the_release() {
        let (root, _) = serve(vec![response(&body())]);
        let dir = temp_dir();
        let result = HttpDownloader.download(&root, &dir.join("file.bin"), Some(SIZE as u64 + 1), &AtomicBool::new(false), &mut |_, _| {});
        assert!(matches!(result, Err(DownloadError::Failed(message)) if message.contains("サイズ")));
    }

    #[test]
    fn stops_when_cancelled() {
        let dir = temp_dir();
        let result = HttpDownloader.download("http://127.0.0.1:9/", &dir.join("file.bin"), None, &AtomicBool::new(true), &mut |_, _| {});
        assert_eq!(result, Err(DownloadError::Cancelled));
    }
}
