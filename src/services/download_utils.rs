use std::io::{Seek, SeekFrom, Write};

use axum::http::{header, HeaderName};
use base64::{engine::general_purpose, Engine};
use bytes::Buf;
use futures::TryStreamExt;
use reqwest::Response;
use tempfile::SpooledTempFile;
use tokio::io::AsyncRead;
use tokio_util::compat::FuturesAsyncReadCompatExt;

pub struct DownloadResult {
    pub response: Response,
    pub filename: String,
    pub filename_ascii: String,
    pub caption: String,
}

pub fn get_response_async_read(it: Response) -> impl AsyncRead {
    it.bytes_stream()
        .map_err(std::io::Error::other)
        .into_async_read()
        .compat()
}

pub async fn response_to_tempfile(res: &mut Response) -> Option<(SpooledTempFile, usize)> {
    let mut tmp_file = tempfile::spooled_tempfile(5 * 1024 * 1024);

    let mut data_size: usize = 0;

    {
        loop {
            let chunk = res.chunk().await;

            let result = match chunk {
                Ok(v) => v,
                Err(_) => return None,
            };

            let data = match result {
                Some(v) => v,
                None => break,
            };

            data_size += data.len();

            match tmp_file.write_all(data.chunk()) {
                Ok(_) => {}
                Err(_) => return None,
            };
        }

        tmp_file.seek(SeekFrom::Start(0)).unwrap();
    }

    Some((tmp_file, data_size))
}

const FALLBACK_FILENAME: &str = "file.bin";

pub fn sanitize_content_disposition_filename(filename_ascii: &str) -> String {
    let sanitized: String = filename_ascii
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .map(|c| match c {
            '"' => "\\\"".to_string(),
            '\\' => "\\\\".to_string(),
            other => other.to_string(),
        })
        .collect();

    if sanitized.trim().is_empty() {
        FALLBACK_FILENAME.to_string()
    } else {
        sanitized
    }
}

pub fn build_download_headers(
    filename: &str,
    filename_ascii: &str,
    caption: &str,
    content_length: Option<u64>,
) -> Vec<(HeaderName, String)> {
    let encoder = general_purpose::STANDARD;
    let sanitized_filename = sanitize_content_disposition_filename(filename_ascii);

    let mut headers = vec![
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{sanitized_filename}\""),
        ),
        (
            HeaderName::from_static("x-filename-b64"),
            encoder.encode(filename),
        ),
        (
            HeaderName::from_static("x-caption-b64"),
            encoder.encode(caption),
        ),
    ];

    if let Some(len) = content_length {
        headers.push((header::CONTENT_LENGTH, len.to_string()));
    }

    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_download_headers_base64_roundtrip() {
        let filename = "книга.pdf";
        let filename_ascii = "book.pdf";
        let caption = "Автор: Тест — Книга";

        let headers = build_download_headers(filename, filename_ascii, caption, None);

        let filename_b64 = headers
            .iter()
            .find(|(name, _)| name == "x-filename-b64")
            .map(|(_, v)| v.clone())
            .expect("x-filename-b64 header missing");
        let caption_b64 = headers
            .iter()
            .find(|(name, _)| name == "x-caption-b64")
            .map(|(_, v)| v.clone())
            .expect("x-caption-b64 header missing");

        let decoded_filename = general_purpose::STANDARD.decode(filename_b64).unwrap();
        let decoded_caption = general_purpose::STANDARD.decode(caption_b64).unwrap();

        assert_eq!(String::from_utf8(decoded_filename).unwrap(), filename);
        assert_eq!(String::from_utf8(decoded_caption).unwrap(), caption);
    }

    #[test]
    fn test_build_download_headers_content_length_present() {
        let headers = build_download_headers("f.pdf", "f.pdf", "cap", Some(12345));

        let content_length = headers
            .iter()
            .find(|(name, _)| name == header::CONTENT_LENGTH)
            .map(|(_, v)| v.clone());

        assert_eq!(content_length, Some("12345".to_string()));
    }

    #[test]
    fn test_build_download_headers_content_length_absent() {
        let headers = build_download_headers("f.pdf", "f.pdf", "cap", None);

        let content_length = headers
            .iter()
            .find(|(name, _)| name == header::CONTENT_LENGTH);

        assert!(content_length.is_none());
    }

    #[test]
    fn test_build_download_headers_content_disposition_escaping() {
        let filename_ascii = "my \"file\"; test.pdf";

        let headers = build_download_headers("filename.pdf", filename_ascii, "cap", None);

        let content_disposition = headers
            .iter()
            .find(|(name, _)| name == header::CONTENT_DISPOSITION)
            .map(|(_, v)| v.clone())
            .expect("content-disposition header missing");

        assert_eq!(
            content_disposition,
            "attachment; filename=\"my \\\"file\\\"; test.pdf\""
        );
    }

    #[test]
    fn test_sanitize_strips_non_ascii_and_control_chars() {
        let input = "café\u{0007}report.pdf";
        let sanitized = sanitize_content_disposition_filename(input);

        assert_eq!(sanitized, "cafreport.pdf");
    }

    #[test]
    fn test_sanitize_fallback_on_empty_input() {
        assert_eq!(sanitize_content_disposition_filename(""), FALLBACK_FILENAME);
    }

    #[test]
    fn test_sanitize_fallback_on_all_non_ascii_input() {
        assert_eq!(
            sanitize_content_disposition_filename("книга"),
            FALLBACK_FILENAME
        );
    }
}
