#[cfg(test)]
use std::io::Cursor;
use std::io::Read;

use zip::write::FileOptions;

pub const MAX_DECOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_COMPRESSION_RATIO: u64 = 100;
pub const ZIP_COMPRESSION_LEVEL: i64 = 6;
pub const MAX_REUSE_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug)]
pub enum ZipError {
    BadArchive,
    Internal(String),
}

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZipError::BadArchive => write!(f, "bad or oversized zip archive"),
            ZipError::Internal(msg) => write!(f, "internal zip error: {}", msg),
        }
    }
}

impl std::error::Error for ZipError {}

/// Extracts the single entry matching `ext` (matched on the file's extension, not a
/// substring) from an in-memory zip archive. Ported from
/// `books_downloader/src/services/downloader/zip.rs::unzip`, adapted to operate purely
/// in memory via `Cursor` instead of `SpooledTempFile`.
#[cfg(test)]
pub fn extract_single_entry(
    archive: &[u8],
    ext: &str,
    max_decompressed_bytes: u64,
    max_compression_ratio: u64,
) -> Result<Vec<u8>, ZipError> {
    let mut zip_archive =
        zip::ZipArchive::new(Cursor::new(archive)).map_err(|_| ZipError::BadArchive)?;

    let ext_lower = ext.to_lowercase();

    for i in 0..zip_archive.len() {
        let mut file = zip_archive.by_index(i).map_err(|_| ZipError::BadArchive)?;
        let filename = file.name().map_err(|_| ZipError::BadArchive)?.into_owned();

        let matches_ext = filename
            .rsplit('.')
            .next()
            .map(|e| e.eq_ignore_ascii_case(&ext_lower))
            .unwrap_or(false);

        if !file.is_dir() && matches_ext {
            let declared_size = file.size();
            let compressed_size = file.compressed_size().max(1);

            if declared_size > max_decompressed_bytes {
                return Err(ZipError::BadArchive);
            }

            if declared_size / compressed_size > max_compression_ratio {
                return Err(ZipError::BadArchive);
            }

            let mut output: Vec<u8> = Vec::new();
            let mut limited = (&mut file).take(max_decompressed_bytes.saturating_add(1));

            let size: usize = match std::io::copy(&mut limited, &mut output) {
                Ok(v) if v > max_decompressed_bytes => return Err(ZipError::BadArchive),
                Ok(v) => v.try_into().map_err(|_| ZipError::BadArchive)?,
                Err(_) => return Err(ZipError::BadArchive),
            };

            debug_assert_eq!(size, output.len());

            return Ok(output);
        }
    }

    Err(ZipError::BadArchive)
}

/// Returns true when `file_type` names a format that is already compressed
/// (zip/epub), so the ZIP entry should be stored rather than re-deflated.
pub fn is_precompressed_file_type(file_type: &str) -> bool {
    let lower = file_type.to_lowercase();
    lower == "zip" || lower == "epub"
}

/// Builds an in-memory zip archive containing a single entry named `entry_name` with
/// `content` as its bytes. Ported from
/// `books_downloader/src/services/downloader/zip.rs::zip`, adapted to operate purely in
/// memory via `Cursor` instead of `SpooledTempFile`.
#[cfg(test)]
pub fn build_single_entry_zip(
    content: &[u8],
    entry_name: &str,
    compression_level: i64,
    stored: bool,
) -> Result<Vec<u8>, ZipError> {
    let output: Vec<u8> = Vec::new();
    let mut archive = zip::ZipWriter::new(Cursor::new(output));

    let options: FileOptions<_> = if stored {
        FileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .unix_permissions(0o755)
    } else {
        FileOptions::default()
            .compression_level(Some(compression_level))
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o755)
    };

    archive
        .start_file::<&str, ()>(entry_name, options)
        .map_err(|_| ZipError::Internal("failed to start zip entry".to_string()))?;

    std::io::copy(&mut Cursor::new(content), &mut archive)
        .map_err(|_| ZipError::Internal("failed to write zip entry".to_string()))?;

    let cursor = archive
        .finish()
        .map_err(|_| ZipError::Internal("failed to finalize zip archive".to_string()))?;

    Ok(cursor.into_inner())
}

/// Copies the source directly into a disk-backed ZIP. When repacking, the selected
/// entry is inflated into the ZIP writer, never into a second large allocation.
pub fn transform_zip_file(
    source: super::temp_file::TempFile,
    entry_name: &str,
    inner_type: &str,
    repack: bool,
) -> Result<super::temp_file::TempFile, Box<dyn std::error::Error + Send + Sync>> {
    let (output, file) = super::temp_file::TempFile::create()?;
    let mut writer = zip::ZipWriter::new(file);
    let options: FileOptions<()> = FileOptions::default()
        .compression_method(if is_precompressed_file_type(inner_type) {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        })
        .compression_level(Some(ZIP_COMPRESSION_LEVEL))
        .unix_permissions(0o755);
    writer.start_file(entry_name, options)?;
    if repack {
        let mut archive = zip::ZipArchive::new(source.open()?)?;
        let mut found = false;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if entry.is_dir()
                || !entry
                    .name()?
                    .rsplit('.')
                    .next()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case(inner_type))
            {
                continue;
            }
            if entry.size() > MAX_DECOMPRESSED_BYTES
                || entry.size() / entry.compressed_size().max(1) > MAX_COMPRESSION_RATIO
            {
                return Err(Box::new(ZipError::BadArchive));
            }
            let size = std::io::copy(
                &mut (&mut entry).take(MAX_DECOMPRESSED_BYTES + 1),
                &mut writer,
            )?;
            if size > MAX_DECOMPRESSED_BYTES {
                return Err(Box::new(ZipError::BadArchive));
            }
            found = true;
            break;
        }
        if !found {
            return Err(Box::new(ZipError::BadArchive));
        }
    } else {
        std::io::copy(&mut source.open()?, &mut writer)?;
    }
    writer.finish()?;
    if output.size()? > MAX_REUSE_BYTES as u64 {
        return Err(Box::new(ZipError::BadArchive));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_wrap_and_repack_preserve_content_and_rename_entry() {
        use std::io::Write;
        let (source, mut file) = super::super::temp_file::TempFile::create().unwrap();
        file.write_all(b"book contents").unwrap();
        drop(file);
        let wrapped = transform_zip_file(source, "old.fb2", "fb2", false).unwrap();
        let repacked = transform_zip_file(wrapped, "new.fb2", "fb2", true).unwrap();
        let mut archive = zip::ZipArchive::new(repacked.open().unwrap()).unwrap();
        assert_eq!(archive.len(), 1);
        let mut entry = archive.by_index(0).unwrap();
        assert_eq!(entry.name().unwrap(), "new.fb2");
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        assert_eq!(content, b"book contents");
    }

    #[test]
    fn disk_repack_rejects_excessive_expansion() {
        use std::io::Write;
        let zipped = build_single_entry_zip(&vec![0; 1024 * 1024], "book.fb2", 6, false).unwrap();
        let (source, mut file) = super::super::temp_file::TempFile::create().unwrap();
        file.write_all(&zipped).unwrap();
        drop(file);
        assert!(transform_zip_file(source, "new.fb2", "fb2", true).is_err());
    }

    const GENEROUS_MAX_DECOMPRESSED: u64 = 5 * 1024 * 1024;
    const GENEROUS_MAX_RATIO: u64 = 1000;

    #[test]
    fn zip_then_unzip_round_trips_content() {
        let original = b"fb2 file contents";

        let zipped =
            build_single_entry_zip(original, "book.fb2", 6, false).expect("zip should succeed");

        let unzipped = extract_single_entry(
            &zipped,
            "fb2",
            GENEROUS_MAX_DECOMPRESSED,
            GENEROUS_MAX_RATIO,
        )
        .expect("unzip should find the fb2 entry");

        assert_eq!(unzipped, original);
    }

    #[test]
    fn selects_entry_by_extension_not_substring() {
        let output: Vec<u8> = Vec::new();
        let mut archive = zip::ZipWriter::new(Cursor::new(output));
        let options: FileOptions<()> = FileOptions::default();

        archive
            .start_file::<&str, ()>("cover.fb2.jpg", options)
            .unwrap();
        std::io::Write::write_all(&mut archive, b"not the fb2 entry").unwrap();

        archive.start_file::<&str, ()>("book.fb2", options).unwrap();
        std::io::Write::write_all(&mut archive, b"fb2 file contents").unwrap();

        let zipped = archive.finish().unwrap().into_inner();

        let unzipped = extract_single_entry(
            &zipped,
            "fb2",
            GENEROUS_MAX_DECOMPRESSED,
            GENEROUS_MAX_RATIO,
        )
        .expect("should find book.fb2 by extension, not cover.fb2.jpg by substring");

        assert_eq!(unzipped, b"fb2 file contents");
    }

    #[test]
    fn directory_entries_are_skipped() {
        let output: Vec<u8> = Vec::new();
        let mut archive = zip::ZipWriter::new(Cursor::new(output));
        let options: FileOptions<()> = FileOptions::default();

        archive.add_directory("fb2", options).unwrap();
        archive.start_file::<&str, ()>("real.fb2", options).unwrap();
        std::io::Write::write_all(&mut archive, b"real fb2 contents").unwrap();

        let zipped = archive.finish().unwrap().into_inner();

        let unzipped = extract_single_entry(
            &zipped,
            "fb2",
            GENEROUS_MAX_DECOMPRESSED,
            GENEROUS_MAX_RATIO,
        )
        .expect("should skip the directory and find real.fb2");

        assert_eq!(unzipped, b"real fb2 contents");
    }

    #[test]
    fn corrupt_zip_bytes_return_bad_archive_instead_of_panicking() {
        let garbage = b"this is not a zip file";

        let result = extract_single_entry(
            garbage,
            "fb2",
            GENEROUS_MAX_DECOMPRESSED,
            GENEROUS_MAX_RATIO,
        );

        assert!(matches!(result, Err(ZipError::BadArchive)));
    }

    #[test]
    fn oversized_declared_entry_is_rejected() {
        let original = vec![b'a'; 2 * 1024 * 1024];

        let zipped =
            build_single_entry_zip(&original, "book.fb2", 6, false).expect("zip should succeed");

        let result = extract_single_entry(&zipped, "fb2", 1024 * 1024, u64::MAX);

        assert!(matches!(result, Err(ZipError::BadArchive)));
    }

    #[test]
    fn high_compression_ratio_entry_is_rejected() {
        let original = vec![0u8; 2 * 1024 * 1024];

        let zipped =
            build_single_entry_zip(&original, "book.fb2", 6, false).expect("zip should succeed");
        assert!(
            zipped.len() < original.len() / 20,
            "test fixture must compress well beyond the ratio cap to be meaningful"
        );

        let result = extract_single_entry(&zipped, "fb2", u64::MAX, 10);

        assert!(matches!(result, Err(ZipError::BadArchive)));
    }

    #[test]
    fn rename_round_trip_preserves_content_with_new_cyrillic_name() {
        let old_name = "OldName_42.fb2";
        let new_name = "НовоеИмя_42.fb2";
        let content = b"some fb2 book bytes to be renamed";

        let old_zip =
            build_single_entry_zip(content, old_name, ZIP_COMPRESSION_LEVEL, false).unwrap();

        let extracted = extract_single_entry(
            &old_zip,
            "fb2",
            MAX_DECOMPRESSED_BYTES,
            MAX_COMPRESSION_RATIO,
        )
        .expect("should extract old entry");
        assert_eq!(extracted, content);

        let new_zip =
            build_single_entry_zip(&extracted, new_name, ZIP_COMPRESSION_LEVEL, false).unwrap();

        let mut reopened = zip::ZipArchive::new(Cursor::new(&new_zip)).unwrap();
        assert_eq!(reopened.len(), 1);

        let mut entry = reopened.by_index(0).unwrap();
        assert_eq!(entry.name().unwrap(), new_name);

        let mut entry_content = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut entry_content).unwrap();
        assert_eq!(entry_content, content);
    }

    #[test]
    fn stored_compression_output_is_readable() {
        let content = b"stored, not deflated";

        let zipped = build_single_entry_zip(content, "book.epub", ZIP_COMPRESSION_LEVEL, true)
            .expect("zip should succeed");

        let unzipped = extract_single_entry(
            &zipped,
            "epub",
            GENEROUS_MAX_DECOMPRESSED,
            GENEROUS_MAX_RATIO,
        )
        .expect("should extract stored entry");

        assert_eq!(unzipped, content);
    }
}
