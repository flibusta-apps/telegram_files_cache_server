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
    caption_title_length = len(caption_title) + 2

    caption_authors_parts = []
    authors_caption_length = 0
    for author in book.authors:
        author_caption = f"👤 {get_author_string(author)}"

        if (
            caption_title_length + authors_caption_length + len(author_caption) + 1
        ) <= 1024:
            caption_authors_parts.append(author_caption)
            authors_caption_length += len(author_caption) + 1
        else:
            break

    if not caption_authors_parts:
        return caption_title

    caption_authors = "\n".join(caption_authors_parts)

    return caption_title + "\n\n" + caption_authors
