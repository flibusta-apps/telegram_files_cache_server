//! Disk-backed fill payloads. No dependency/lockfile changes are needed.
use std::{fs::OpenOptions, io, path::PathBuf};
use tokio::io::AsyncReadExt;

pub struct TempFile {
    path: PathBuf,
}

impl TempFile {
    pub fn create() -> io::Result<(Self, std::fs::File)> {
        loop {
            let path = std::env::temp_dir().join(format!(
                "telegram-cache-{}-{:032x}",
                std::process::id(),
                rand::random::<u128>()
            ));
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => return Ok((Self { path }, file)),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        }
    }

    pub fn open(&self) -> io::Result<std::fs::File> {
        std::fs::File::open(&self.path)
    }

    pub fn size(&self) -> io::Result<u64> {
        Ok(self.open()?.metadata()?.len())
    }

    /// Reopen on every attempt, so connect-only retries start at offset zero.
    pub async fn body(&self) -> io::Result<reqwest::Body> {
        let file = tokio::fs::File::open(&self.path).await?;
        let stream = futures::stream::try_unfold(file, |mut file| async move {
            let mut buffer = vec![0; 64 * 1024];
            let size = file.read(&mut buffer).await?;
            if size == 0 {
                return Ok::<_, io::Error>(None);
            }
            buffer.truncate(size);
            Ok(Some((bytes::Bytes::from(buffer), file)))
        });
        Ok(reqwest::Body::wrap_stream(stream))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if let Err(err) = std::fs::remove_file(&self.path) {
            tracing::warn!(error = %err, "failed to remove fill temporary file");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn temporary_file_is_reopenable_and_removed_on_drop() {
        let (guard, mut file) = TempFile::create().unwrap();
        file.write_all(b"payload").unwrap();
        drop(file);
        let path = guard.path.clone();
        assert_eq!(guard.size().unwrap(), 7);
        assert_eq!(std::fs::read(&path).unwrap(), b"payload");
        drop(guard);
        assert!(!path.exists());
    }
}
