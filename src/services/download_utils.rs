use futures::TryStreamExt;
use reqwest::Response;
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
