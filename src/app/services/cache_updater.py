import logging
from typing import Optional

from arq.connections import ArqRedis

from app.models import CachedFile
from app.services.caption_getter import get_caption
from app.services.downloader import download
from app.services.files_client import upload_file
from app.services.library_client import get_books, get_book, Book


logger = logging.getLogger("telegram_channel_files_manager")


PAGE_SIZE = 100


class FileTypeNotAllowed(Exception):
    def __init__(self, message: str) -> None:
        super().__init__(message)


async def check_book(book: Book, arq_pool: ArqRedis) -> None:
    for file_type in book.available_types:
        exists = await CachedFile.objects.filter(
            object_id=book.id, object_type=file_type
        ).exists()

        if not exists:
            await arq_pool.enqueue_job("cache_file_by_book_id", book.id, file_type)


async def check_books_page(ctx, page_number: int) -> None:
    arq_pool: ArqRedis = ctx["arc_pool"]

    page = await get_books(page_number, PAGE_SIZE)

    for book in page.items:
        await check_book(book, arq_pool)


async def check_books(ctx) -> None:
    arq_pool: ArqRedis = ctx["arc_pool"]
    books_page = await get_books(1, PAGE_SIZE)

    for page_number in range(1, books_page.total_pages + 1):
        await arq_pool.enqueue_job("check_books_page", page_number)


async def cache_file(book: Book, file_type) -> Optional[CachedFile]:
    logger.info(f"Cache {book.id} {file_type}...")
    data = await download(book.source.id, book.remote_id, file_type)

    if data is None:
        return None

    content, filename = data

    caption = get_caption(book)

    upload_data = await upload_file(content, filename, caption)

    return await CachedFile.objects.create(
        object_id=book.id, object_type=file_type, data=upload_data.data
    )


async def cache_file_by_book_id(
    ctx, book_id: int, file_type: str
) -> Optional[CachedFile]:
    book = await get_book(book_id)

    if file_type not in book.available_types:
        raise FileTypeNotAllowed(f"{file_type} not in {book.available_types}!")

    return await cache_file(book, file_type)