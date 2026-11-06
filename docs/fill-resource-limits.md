# Cache fill resource limits

`MAX_CONCURRENT_FILLS` limits active fills across HTTP misses and warmup together.
The default is `4`; zero, invalid, or out-of-range values fall back to `4`.
Warmup also retains its existing `CACHE_WARMUP_CONCURRENCY` limit.

Downloaded fill payloads are always spooled to disk, including small files, rather
than retaining up to 512 MiB per miss in memory. Multipart uploads stream these
files in 64 KiB chunks and reopen them from the start for connect-only retries.
ZIP reuse streams the selected entry directly into a new disk-backed ZIP; the
compressed, unpacked, and repacked book are not materialized together in RAM.
Existing download, decompressed-size, and compression-ratio caps still apply.

Temporary files use the system temporary directory (`TMPDIR` on Unix), exclusive
creation, owner-only Unix permissions, and a Drop guard for cleanup after success
or errors. Provision writable disk space for concurrent source and output files;
do not point `TMPDIR` at a RAM-backed filesystem if avoiding RAM pressure is the
goal. Abrupt process termination cannot run Drop cleanup and may leave files named
`telegram-cache-<pid>-<random>` for operational cleanup. Disk I/O errors fail the
fill (or allow reuse to fall back to the cold path), rather than falling back to
large memory allocations.
