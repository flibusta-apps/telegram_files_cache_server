use serde::Deserialize;

/// Allowlist of `object_type` values accepted by the API. Mirrors the formats accepted by
/// `books_downloader` (converted targets plus source formats).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectType {
    Fb2,
    Fb2zip,
    Epub,
    Mobi,
    Html,
    Pdf,
    Djvu,
    Txt,
    Rtf,
    Doc,
    Docx,
}

impl ObjectType {
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectType::Fb2 => "fb2",
            ObjectType::Fb2zip => "fb2zip",
            ObjectType::Epub => "epub",
            ObjectType::Mobi => "mobi",
            ObjectType::Html => "html",
            ObjectType::Pdf => "pdf",
            ObjectType::Djvu => "djvu",
            ObjectType::Txt => "txt",
            ObjectType::Rtf => "rtf",
            ObjectType::Doc => "doc",
            ObjectType::Docx => "docx",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Result<ObjectType, serde_json::Error> {
        serde_json::from_str(&serde_json::to_string(raw).unwrap())
    }

    #[test]
    fn accepts_allowlisted_and_round_trips() {
        for t in [
            ObjectType::Fb2,
            ObjectType::Fb2zip,
            ObjectType::Epub,
            ObjectType::Mobi,
            ObjectType::Html,
            ObjectType::Pdf,
            ObjectType::Djvu,
            ObjectType::Txt,
            ObjectType::Rtf,
            ObjectType::Doc,
            ObjectType::Docx,
        ] {
            assert_eq!(parse(t.as_str()).unwrap(), t);
        }
    }

    #[test]
    fn rejects_garbage() {
        for s in ["", "../admin", "epub?x=y", "EPUB", "book", "author"] {
            assert!(parse(s).is_err(), "{s}");
        }
    }
}
