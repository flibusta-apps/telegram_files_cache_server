use serde::Deserialize;

// Mirrors the query structs in src/views.rs. Kept local so the test
// stays a pure unit test (no app wiring, no DB, no live services).

#[derive(Deserialize, Debug, PartialEq)]
struct GetCachedFileQuery {
    pub copy: bool,
    #[serde(default)]
    pub normalized: Option<bool>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct DownloadCachedFileQuery {
    #[serde(default)]
    pub normalized: Option<bool>,
}

#[derive(Deserialize, Debug, PartialEq)]
struct DeleteCachedFileQuery {
    #[serde(default)]
    pub normalized: Option<bool>,
}

fn parse<T: for<'de> Deserialize<'de>>(s: &str) -> Result<T, serde_urlencoded::de::Error> {
    serde_urlencoded::from_str(s)
}

#[test]
fn get_query_explicit_true() {
    let q: GetCachedFileQuery = parse("copy=true&normalized=true").unwrap();
    assert_eq!(
        q,
        GetCachedFileQuery {
            copy: true,
            normalized: Some(true)
        }
    );
}

#[test]
fn get_query_explicit_false() {
    let q: GetCachedFileQuery = parse("copy=false&normalized=false").unwrap();
    assert_eq!(
        q,
        GetCachedFileQuery {
            copy: false,
            normalized: Some(false)
        }
    );
}

#[test]
fn get_query_normalized_absent() {
    let q: GetCachedFileQuery = parse("copy=true").unwrap();
    assert_eq!(
        q,
        GetCachedFileQuery {
            copy: true,
            normalized: None
        }
    );
}

#[test]
fn get_query_normalized_invalid() {
    assert!(parse::<GetCachedFileQuery>("copy=true&normalized=foo").is_err());
    assert!(parse::<GetCachedFileQuery>("copy=true&normalized=1").is_err());
    assert!(parse::<GetCachedFileQuery>("copy=true&normalized=").is_err());
}

#[test]
fn download_query_explicit_true() {
    let q: DownloadCachedFileQuery = parse("normalized=true").unwrap();
    assert_eq!(
        q,
        DownloadCachedFileQuery {
            normalized: Some(true)
        }
    );
}

#[test]
fn download_query_explicit_false() {
    let q: DownloadCachedFileQuery = parse("normalized=false").unwrap();
    assert_eq!(
        q,
        DownloadCachedFileQuery {
            normalized: Some(false)
        }
    );
}

#[test]
fn download_query_normalized_absent() {
    let q: DownloadCachedFileQuery = parse("").unwrap();
    assert_eq!(q, DownloadCachedFileQuery { normalized: None });
}

#[test]
fn download_query_normalized_invalid() {
    assert!(parse::<DownloadCachedFileQuery>("normalized=foo").is_err());
    assert!(parse::<DownloadCachedFileQuery>("normalized=yes").is_err());
    assert!(parse::<DownloadCachedFileQuery>("normalized=").is_err());
}

#[test]
fn delete_query_explicit_true() {
    let q: DeleteCachedFileQuery = parse("normalized=true").unwrap();
    assert_eq!(
        q,
        DeleteCachedFileQuery {
            normalized: Some(true)
        }
    );
}

#[test]
fn delete_query_explicit_false() {
    let q: DeleteCachedFileQuery = parse("normalized=false").unwrap();
    assert_eq!(
        q,
        DeleteCachedFileQuery {
            normalized: Some(false)
        }
    );
}

#[test]
fn delete_query_normalized_absent() {
    let q: DeleteCachedFileQuery = parse("").unwrap();
    assert_eq!(q, DeleteCachedFileQuery { normalized: None });
}

#[test]
fn delete_query_normalized_invalid() {
    assert!(parse::<DeleteCachedFileQuery>("normalized=foo").is_err());
    assert!(parse::<DeleteCachedFileQuery>("normalized=1").is_err());
    assert!(parse::<DeleteCachedFileQuery>("normalized=").is_err());
}
