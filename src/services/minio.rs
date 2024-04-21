use std::io::Read;

use async_stream::stream;
use bytes::Bytes;
use minio_rsc::{client::PresignedArgs, error::Error, provider::StaticProvider, Minio};
use tempfile::SpooledTempFile;

use crate::config;

pub fn get_minio() -> Minio {
    let provider = StaticProvider::new(
        &config::CONFIG.minio_access_key,
        &config::CONFIG.minio_secret_key,
        None,
    );

    Minio::builder()
        .endpoint(&config::CONFIG.minio_host)
        .provider(provider)
        .secure(true)
        .build()
        .unwrap()
}

pub fn get_internal_minio() -> Minio {
    let provider = StaticProvider::new(
        &config::CONFIG.minio_access_key,
        &config::CONFIG.minio_secret_key,
        None,
    );

    Minio::builder()
        .endpoint(&config::CONFIG.internal_minio_host)
        .provider(provider)
        .secure(false)
        .build()
        .unwrap()
}

pub fn get_stream(
    mut temp_file: Box<dyn Read + Send>,
) -> impl futures_core::Stream<Item = Result<Bytes, Error>> {
    stream! {
        let mut buf = [0; 2048];

        while let Ok(count) = temp_file.read(&mut buf) {
            if count == 0 {
                break;
            }

            yield Ok(Bytes::copy_from_slice(&buf[0..count]))
        }
    }
}

pub async fn upload_to_minio(
    archive: SpooledTempFile,
    filename: String,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let internal_minio = get_internal_minio();

    let is_bucket_exist = match internal_minio
        .bucket_exists(&config::CONFIG.minio_bucket)
        .await
    {
        Ok(v) => v,
        Err(err) => return Err(Box::new(err)),
    };

    if !is_bucket_exist {
        let _ = internal_minio
            .make_bucket(&config::CONFIG.minio_bucket, false)
            .await;
    }

    let data_stream = get_stream(Box::new(archive));

    if let Err(err) = internal_minio
        .put_object_stream(
            &config::CONFIG.minio_bucket,
            filename.clone(),
            Box::pin(data_stream),
            None,
        )
        .await
    {
        return Err(Box::new(err));
    }

    let minio = get_minio();

    let link = match minio
        .presigned_get_object(PresignedArgs::new(&config::CONFIG.minio_bucket, filename))
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return Err(Box::new(err));
        }
    };

    Ok(link)
}
