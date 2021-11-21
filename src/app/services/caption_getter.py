from app.services.library_client import Book, BookAuthor


def get_author_string(author: BookAuthor) -> str:
    author_parts = []

    if author.last_name:
        author_parts.append(author.last_name)
    
    if author.first_name:
        author_parts.append(author.first_name)
    
    if author.middle_name:
        author_parts.append(author.middle_name)

    return " ".join(author_parts)


def get_caption(book: Book) -> str:
    caption_title = f"📖 {book.title}"

    caption_authors_parts = []
    for author in book.authors:
        caption_authors_parts.append(
            f"👤 {get_author_string(author)}"
        )
    
    if not caption_authors_parts:
        return caption_title
    
    caption_authors = "\n".join(caption_authors_parts)

    return caption_title + "\n\n" + caption_authors
