use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct Source {
    pub id: u32,
    // name: String
}

#[derive(Deserialize, Debug, Clone)]
pub struct BookAuthor {
    pub id: u32,
    pub first_name: String,
    pub last_name: String,
    pub middle_name: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct BookWithRemote {
    pub id: u32,
    pub remote_id: u32,
    pub title: String,
    pub lang: String,
    pub file_type: String,
    pub uploaded: String,
    pub authors: Vec<BookAuthor>,
    pub source: Source,
}

#[derive(Deserialize, Debug, Clone)]
pub struct BaseBook {
    pub id: i32,
    pub available_types: Vec<String>,
}

impl BookAuthor {
    pub fn get_caption(self) -> String {
        let mut parts: Vec<String> = vec![];

        if !self.last_name.is_empty() {
            parts.push(self.last_name);
        }

        if !self.first_name.is_empty() {
            parts.push(self.first_name);
        }

        if !self.middle_name.is_empty() {
            parts.push(self.middle_name);
        }

        let joined_parts = parts.join(" ");

        format!("👤 {joined_parts}")
    }
}

impl BookWithRemote {
    pub fn get_caption(self) -> String {
        let BookWithRemote { title, authors, .. } = self;

        let caption_title = format!("📖 {title}");

        let author_captions: Vec<String> = authors.into_iter().map(|a| a.get_caption()).collect();

        let mut author_parts: Vec<String> = vec![];
        let mut author_parts_len = 3;

        for author_caption in author_captions {
            if caption_title.len() + author_parts_len + author_caption.len() < 1024 {
                author_parts_len += author_caption.len() + 1;
                author_parts.push(author_caption);
            } else {
                break;
            }
        }

        let caption_authors = author_parts.join("\n");

        format!("{caption_title}\n\n{caption_authors}")
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u32,

    pub page: u32,

    pub size: u32,
    pub pages: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn author(first: &str, last: &str, middle: &str) -> BookAuthor {
        BookAuthor {
            id: 1,
            first_name: first.to_string(),
            last_name: last.to_string(),
            middle_name: middle.to_string(),
        }
    }

    fn book(title: &str, authors: Vec<BookAuthor>) -> BookWithRemote {
        BookWithRemote {
            id: 1,
            remote_id: 1,
            title: title.to_string(),
            lang: "en".to_string(),
            file_type: "fb2".to_string(),
            uploaded: "2024-01-01".to_string(),
            authors,
            source: Source { id: 1 },
        }
    }

    #[test]
    fn book_author_get_caption_all_parts_present() {
        let a = author("First", "Last", "Middle");
        assert_eq!(a.get_caption(), "👤 Last First Middle");
    }

    #[test]
    fn book_author_get_caption_skips_empty_parts_no_extra_whitespace() {
        let a = author("OnlyFirst", "", "");
        assert_eq!(a.get_caption(), "👤 OnlyFirst");
    }

    #[test]
    fn book_with_remote_get_caption_short_book_full_caption() {
        let b = book(
            "Short Title",
            vec![author("Jane", "Doe", ""), author("John", "Smith", "Middle")],
        );

        let caption = b.get_caption();

        assert_eq!(
            caption,
            "📖 Short Title\n\n👤 Doe Jane\n👤 Smith John Middle"
        );
    }

    #[test]
    fn book_with_remote_get_caption_truncates_after_byte_budget() {
        // Each author caption is exactly 55 bytes: "👤 " (4-byte emoji + 1 space = 5
        // bytes) + a 50-byte name (3-digit index prefix + 47 'X' padding).
        // With a 1-byte title ("📖 T" = 6 bytes) and the loop's running-length
        // accounting (start at 3, +55+1=56 per pushed author), the current
        // implementation pushes exactly the first 18 of 25 authors before the
        // 1024-byte budget check fails on the 19th.
        let authors: Vec<BookAuthor> = (0..25)
            .map(|i| {
                let name = format!("{:03}{}", i, "X".repeat(47));
                author(&name, "", "")
            })
            .collect();

        let b = book("T", authors);
        let caption = b.get_caption();

        for i in 0..18 {
            let expected_fragment = format!("{:03}{}", i, "X".repeat(47));
            assert!(
                caption.contains(&expected_fragment),
                "expected caption to contain author {i} (within budget)"
            );
        }

        for i in 18..25 {
            let excluded_fragment = format!("{:03}{}", i, "X".repeat(47));
            assert!(
                !caption.contains(&excluded_fragment),
                "expected caption to NOT contain author {i} (over budget)"
            );
        }
    }
}
