use reqwest::StatusCode;

use crate::{config, repository::CachedFileRepository, serializers::CachedFile, views::Database};

use super::{
    downloader::get_filename,
    telegram_files::{
        download_from_telegram_files, upload_bytes_to_telegram_files, UploadData, UploadPayload,
    },
    zip_utils::{
        build_single_entry_zip, extract_single_entry, is_precompressed_file_type,
        MAX_COMPRESSION_RATIO, MAX_DECOMPRESSED_BYTES, MAX_REUSE_BYTES, ZIP_COMPRESSION_LEVEL,
    },
};

/// Describes how the opposite `is_normalized` variant's already-cached Telegram bytes
/// can be turned into the requested variant without re-fetching from the (slow)
/// external source mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReuseKind {
    /// Content is fully independent of `is_normalized`; only the delivered/displayed
    /// filename differs. The opposite variant's bytes can be reused verbatim.
    IdenticalBytes,
    /// The object type bakes the filename into the content itself (as a zip entry
    /// name), so the opposite variant's bytes must be unzipped and re-zipped under the
    /// target filename.
    RepackZip { inner_type: &'static str },
}

/// Default-deny allowlist of object types eligible for cross-normalized reuse.
/// `html` is deliberately excluded: its cold path names zip archives with
/// `force_zip=true` (producing a `.html.zip` document name) while `/filename` always
/// resolves the non-zip (`.html`) name, so reuse would produce a wrongly-named
/// document for that type.
pub fn reuse_kind(object_type: &str) -> Option<ReuseKind> {
    match object_type {
        "fb2" | "epub" | "mobi" => Some(ReuseKind::IdenticalBytes),
        "fb2zip" => Some(ReuseKind::RepackZip { inner_type: "fb2" }),
        _ => None,
    }
}

pub enum ReuseOutcome {
    Uploaded(UploadData),
    NotEligible,
}

/// Attempts to satisfy a cache miss for `(object_id, object_type, is_normalized)` by
/// reusing the already-cached Telegram bytes of the opposite `is_normalized` variant,
/// instead of falling back to a full cold rebuild (external source mirror fetch +
/// Telegram upload). Never calls `cache_file`/`get_cached_file_or_cache` (would
/// double-single-flight / risk cold-build recursion) and never evicts any cache row —
/// eviction remains the sole responsibility of `download_from_cache`.
pub async fn try_reuse_from_opposite_normalized(
    object_id: i32,
    object_type: &str,
    is_normalized: bool,
    db: &Database,
    caption: &str,
    max_retries: u32,
) -> Result<ReuseOutcome, Box<dyn std::error::Error + Send + Sync>> {
    if !config::CONFIG.cross_normalized_reuse {
        tracing::debug!("cross_normalized_reuse disabled by config");
        metrics::counter!("cross_normalized_reuse_total", "outcome" => "disabled").increment(1);
        return Ok(ReuseOutcome::NotEligible);
    }

    let Some(kind) = reuse_kind(object_type) else {
        tracing::debug!(
            object_type,
            "cross_normalized_reuse: object_type not eligible for reuse"
        );
        metrics::counter!("cross_normalized_reuse_total", "outcome" => "unsupported_type")
            .increment(1);
        return Ok(ReuseOutcome::NotEligible);
    };

    let source = match CachedFileRepository::new(db.clone())
        .find_by_object_id_object_type_is_normalized(
            object_id,
            object_type.to_string(),
            !is_normalized,
        )
        .await
    {
        Ok(Some(v)) => v,
        Ok(None) => {
            tracing::debug!(
                object_id,
                object_type,
                is_normalized,
                "cross_normalized_reuse: no opposite-normalized source cached"
            );
            metrics::counter!("cross_normalized_reuse_total", "outcome" => "no_source")
                .increment(1);
            return Ok(ReuseOutcome::NotEligible);
        }
        Err(err) => {
            tracing::warn!(
                object_id,
                object_type,
                is_normalized,
                error = ?err,
                "cross_normalized_reuse: failed to look up opposite-normalized source"
            );
            metrics::counter!("cross_normalized_reuse_total", "outcome" => "db_error").increment(1);
            return Ok(ReuseOutcome::NotEligible);
        }
    };

    if source.is_normalized == is_normalized {
        tracing::warn!(
            object_id,
            object_type,
            is_normalized,
            "cross_normalized_reuse: repository returned a source row with the same \
             is_normalized value as the target; skipping reuse"
        );
        metrics::counter!("cross_normalized_reuse_total", "outcome" => "db_error").increment(1);
        return Ok(ReuseOutcome::NotEligible);
    }

    reuse_from_source(
        &source,
        object_id,
        object_type,
        is_normalized,
        kind,
        caption,
        max_retries,
    )
    .await
}

pub(crate) async fn reuse_from_source(
    source: &CachedFile,
    object_id: i32,
    object_type: &str,
    is_normalized: bool,
    kind: ReuseKind,
    caption: &str,
    max_retries: u32,
) -> Result<ReuseOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let inner_type = match kind {
        ReuseKind::RepackZip { inner_type } => Some(inner_type),
        ReuseKind::IdenticalBytes => None,
    };

    let inner_filename_fut = async {
        match inner_type {
            Some(inner_type) => Some(
                get_filename(
                    object_id,
                    inner_type.to_string(),
                    is_normalized,
                    max_retries,
                )
                .await,
            ),
            None => None,
        }
    };

    let (download_result, target_filename_result, inner_filename_result) = tokio::join!(
        download_from_telegram_files(source.message_id, source.chat_id, max_retries),
        get_filename(
            object_id,
            object_type.to_string(),
            is_normalized,
            max_retries
        ),
        inner_filename_fut,
    );

    let response = match download_result {
        Ok(resp) if resp.status() == StatusCode::NO_CONTENT => {
            tracing::warn!(
                object_id,
                object_type,
                "cross_normalized_reuse: opposite-normalized source returned no content"
            );
            metrics::counter!("cross_normalized_reuse_total", "outcome" => "fetch_failed")
                .increment(1);
            return Ok(ReuseOutcome::NotEligible);
        }
        Ok(resp) => resp,
        Err(err) => {
            tracing::warn!(
                object_id,
                object_type,
                error = ?err,
                "cross_normalized_reuse: failed to fetch opposite-normalized source bytes"
            );
            metrics::counter!("cross_normalized_reuse_total", "outcome" => "fetch_failed")
                .increment(1);
            return Ok(ReuseOutcome::NotEligible);
        }
    };

    let source_bytes = match response.bytes().await {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                object_id,
                object_type,
                error = ?err,
                "cross_normalized_reuse: failed to buffer opposite-normalized source bytes"
            );
            metrics::counter!("cross_normalized_reuse_total", "outcome" => "fetch_failed")
                .increment(1);
            return Ok(ReuseOutcome::NotEligible);
        }
    };

    if source_bytes.is_empty() || source_bytes.len() > MAX_REUSE_BYTES {
        tracing::warn!(
            object_id,
            object_type,
            size = source_bytes.len(),
            "cross_normalized_reuse: opposite-normalized source bytes empty or too large"
        );
        metrics::counter!("cross_normalized_reuse_total", "outcome" => "fetch_failed").increment(1);
        return Ok(ReuseOutcome::NotEligible);
    }

    let target_filename = match target_filename_result {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                object_id,
                object_type,
                error = ?err,
                "cross_normalized_reuse: failed to resolve target filename"
            );
            metrics::counter!("cross_normalized_reuse_total", "outcome" => "filename_failed")
                .increment(1);
            return Ok(ReuseOutcome::NotEligible);
        }
    };

    let inner_filename = match inner_filename_result {
        Some(Ok(v)) => Some(v),
        Some(Err(err)) => {
            tracing::warn!(
                object_id,
                object_type,
                error = ?err,
                "cross_normalized_reuse: failed to resolve inner zip entry filename"
            );
            metrics::counter!("cross_normalized_reuse_total", "outcome" => "filename_failed")
                .increment(1);
            return Ok(ReuseOutcome::NotEligible);
        }
        None => None,
    };

    let document_filename = target_filename.filename_ascii;

    let final_bytes = match kind {
        ReuseKind::IdenticalBytes => source_bytes.to_vec(),
        ReuseKind::RepackZip { inner_type } => {
            let inner_filename = inner_filename
                .expect("inner_filename must be resolved for RepackZip reuse kind")
                .filename;

            let stored = is_precompressed_file_type(inner_type);
            let repack_result = tokio::task::spawn_blocking(move || {
                let extracted = extract_single_entry(
                    &source_bytes,
                    inner_type,
                    MAX_DECOMPRESSED_BYTES,
                    MAX_COMPRESSION_RATIO,
                )?;
                build_single_entry_zip(&extracted, &inner_filename, ZIP_COMPRESSION_LEVEL, stored)
            })
            .await;

            match repack_result {
                Ok(Ok(bytes)) => bytes,
                Ok(Err(err)) => {
                    tracing::warn!(
                        object_id,
                        object_type,
                        error = ?err,
                        "cross_normalized_reuse: failed to repack zip entry"
                    );
                    metrics::counter!("cross_normalized_reuse_total", "outcome" => "repack_failed")
                        .increment(1);
                    return Ok(ReuseOutcome::NotEligible);
                }
                Err(err) => {
                    tracing::warn!(
                        object_id,
                        object_type,
                        error = ?err,
                        "cross_normalized_reuse: repack task panicked/was cancelled"
                    );
                    metrics::counter!("cross_normalized_reuse_total", "outcome" => "repack_failed")
                        .increment(1);
                    return Ok(ReuseOutcome::NotEligible);
                }
            }
        }
    };

    let upload_result = upload_bytes_to_telegram_files(
        UploadPayload {
            bytes: final_bytes.into(),
            filename: document_filename,
            caption: caption.to_string(),
        },
        max_retries,
    )
    .await?;

    metrics::counter!("cross_normalized_reuse_total", "outcome" => "reused").increment(1);

    Ok(ReuseOutcome::Uploaded(upload_result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuse_kind_is_default_deny() {
        assert_eq!(reuse_kind("fb2"), Some(ReuseKind::IdenticalBytes));
        assert_eq!(reuse_kind("epub"), Some(ReuseKind::IdenticalBytes));
        assert_eq!(reuse_kind("mobi"), Some(ReuseKind::IdenticalBytes));
        assert_eq!(
            reuse_kind("fb2zip"),
            Some(ReuseKind::RepackZip { inner_type: "fb2" })
        );

        assert_eq!(reuse_kind("html"), None);
        assert_eq!(reuse_kind(""), None);
        assert_eq!(reuse_kind("FB2"), None);
        assert_eq!(reuse_kind("book"), None);
        assert_eq!(reuse_kind("../admin"), None);
    }
}
