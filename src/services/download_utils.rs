use std::io::{Write, Seek, SeekFrom};

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
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
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

            match tmp_file.write(data.chunk()) {
                Ok(_) => (),
                Err(_) => return None,
            }
        }

        tmp_file.seek(SeekFrom::Start(0)).unwrap();
    }

    Some((tmp_file, data_size))
}
