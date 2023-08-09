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
pub struct Book {
    pub id: u32,
    pub title: String,
    pub lang: String,
    pub file_type: String,
    pub uploaded: String,
    pub authors: Vec<BookAuthor>,
    pub source: Source,
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
    pub available_types: Vec<String>
}

impl BookWithRemote {
    pub fn from_book(book: Book, remote_id: u32) -> Self {
        Self {
            id: book.id,
            remote_id,
            title: book.title,
            lang: book.lang,
            file_type: book.file_type,
            uploaded: book.uploaded,
            authors: book.authors,
            source: book.source,
        }
    }
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
        let BookWithRemote {
            title,
            authors,
            ..
        } = self;

        let caption_title = format!("📖 {title}");

        let author_captions: Vec<String> = authors
            .into_iter()
            .map(|a| a.get_caption())
            .collect();

        let mut author_parts: Vec<String> = vec![];
        let mut author_parts_len = 3;

        for author_caption in author_captions {
            if caption_title.len() + author_parts_len + author_caption.len() + 1 <= 1024 {
                author_parts_len = author_caption.len() + 1;
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
