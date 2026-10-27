use reqwest::StatusCode;

use crate::{config, repository::CachedFileRepository, serializers::CachedFile, views::Database};

use super::{
    download_utils::read_body_capped,
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
    /// The source row is a plain (non-zip) object of `inner_type` for the same
    /// `object_id`; its bytes are wrapped into a single-entry zip named after the
    /// target `inner_type` filename.
    WrapZip { inner_type: &'static str },
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

/// Fallback used when the opposite-normalized source of the *same* object type is not
/// cached: a plain cached object of another type that can be wrapped into the
/// requested type. Currently only `fb2zip` (= `fb2` inside a single-entry zip).
pub fn wrap_fallback(object_type: &str) -> Option<ReuseKind> {
    match object_type {
        "fb2zip" => Some(ReuseKind::WrapZip { inner_type: "fb2" }),
        _ => None,
    }
}

/// Preference order of `is_normalized` values when looking up a wrap source: the same
/// variant first, then the opposite one.
fn wrap_source_preference(is_normalized: bool) -> [bool; 2] {
    [is_normalized, !is_normalized]
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
            return try_wrap_from_plain(
                object_id,
                object_type,
                is_normalized,
                db,
                caption,
                max_retries,
            )
            .await;
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

/// Fallback for `fb2zip`: wraps an already-cached `fb2` row (same `is_normalized`
/// preferred, then opposite) of the same `object_id` into a single-entry zip.
/// Any failure results in `NotEligible` so the caller falls back to the cold path.
async fn try_wrap_from_plain(
    object_id: i32,
    object_type: &str,
    is_normalized: bool,
    db: &Database,
    caption: &str,
    max_retries: u32,
) -> Result<ReuseOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let Some(kind) = wrap_fallback(object_type) else {
        return Ok(ReuseOutcome::NotEligible);
    };
    let ReuseKind::WrapZip { inner_type } = kind else {
        return Ok(ReuseOutcome::NotEligible);
    };

    let repo = CachedFileRepository::new(db.clone());
    let mut source = None;
    for candidate in wrap_source_preference(is_normalized) {
        match repo
            .find_by_object_id_object_type_is_normalized(
                object_id,
                inner_type.to_string(),
                candidate,
            )
            .await
        {
            Ok(Some(v)) => {
                source = Some(v);
                break;
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    object_id,
                    object_type,
                    inner_type,
                    error = ?err,
                    "cross_normalized_reuse: failed to look up wrap source"
                );
                metrics::counter!("cross_normalized_reuse_total", "outcome" => "db_error")
                    .increment(1);
                return Ok(ReuseOutcome::NotEligible);
            }
        }
    }

    let Some(source) = source else {
        tracing::debug!(
            object_id,
            object_type,
            inner_type,
            "cross_normalized_reuse: no wrap source cached"
        );
        metrics::counter!("cross_normalized_reuse_total", "outcome" => "no_source").increment(1);
        return Ok(ReuseOutcome::NotEligible);
    };

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
        ReuseKind::RepackZip { inner_type } | ReuseKind::WrapZip { inner_type } => Some(inner_type),
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

    let source_bytes = match read_body_capped(response, MAX_REUSE_BYTES).await {
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
        ReuseKind::IdenticalBytes => source_bytes,
        ReuseKind::WrapZip { inner_type } => {
            let inner_filename = inner_filename
                .expect("inner_filename must be resolved for WrapZip reuse kind")
                .filename;

            let stored = is_precompressed_file_type(inner_type);
            let wrap_result = tokio::task::spawn_blocking(move || {
                build_single_entry_zip(
                    &source_bytes,
                    &inner_filename,
                    ZIP_COMPRESSION_LEVEL,
                    stored,
                )
            })
            .await;

            match wrap_result {
                Ok(Ok(bytes)) => bytes::Bytes::from(bytes),
                Ok(Err(err)) => {
                    tracing::warn!(
                        object_id,
                        object_type,
                        error = ?err,
                        "cross_normalized_reuse: failed to wrap source into zip"
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
                        "cross_normalized_reuse: wrap task panicked/was cancelled"
                    );
                    metrics::counter!("cross_normalized_reuse_total", "outcome" => "repack_failed")
                        .increment(1);
                    return Ok(ReuseOutcome::NotEligible);
                }
            }
        }
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
                Ok(Ok(bytes)) => bytes::Bytes::from(bytes),
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
            bytes: final_bytes,
            filename: document_filename,
            caption: caption.to_string(),
        },
        max_retries,
    )
    .await?;

    match kind {
        ReuseKind::WrapZip { .. } => {
            tracing::info!(
                object_id,
                object_type,
                source_type = source.object_type,
                source_is_normalized = source.is_normalized,
                "cross_normalized_reuse: wrapped_from_fb2"
            );
            metrics::counter!("cross_normalized_reuse_total", "outcome" => "wrapped_from_fb2")
                .increment(1);
        }
        _ => {
            metrics::counter!("cross_normalized_reuse_total", "outcome" => "reused").increment(1);
        }
    }

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

    #[test]
    fn wrap_fallback_only_for_fb2zip() {
        assert_eq!(
            wrap_fallback("fb2zip"),
            Some(ReuseKind::WrapZip { inner_type: "fb2" })
        );
        for t in ["fb2", "epub", "mobi", "html", "", "FB2ZIP"] {
            assert_eq!(wrap_fallback(t), None, "{t}");
        }
    }

    #[test]
    fn wrap_source_prefers_same_normalization() {
        assert_eq!(wrap_source_preference(true), [true, false]);
        assert_eq!(wrap_source_preference(false), [false, true]);
    }

    #[test]
    fn wrapping_produces_single_entry_zip_with_exact_bytes() {
        use std::io::Read;

        let content = b"<?xml version=\"1.0\"?><FictionBook>hello \xd0\x9f</FictionBook>".to_vec();
        let stored = is_precompressed_file_type("fb2");
        assert!(!stored);
        let zipped =
            build_single_entry_zip(&content, "book.fb2", ZIP_COMPRESSION_LEVEL, stored).unwrap();

        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zipped)).unwrap();
        assert_eq!(archive.len(), 1);
        let mut entry = archive.by_index(0).unwrap();
        assert_eq!(entry.name(), "book.fb2");
        let mut out = Vec::new();
        entry.read_to_end(&mut out).unwrap();
        assert_eq!(out, content);
    }
}
