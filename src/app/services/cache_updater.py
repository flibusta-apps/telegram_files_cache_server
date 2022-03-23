import collections
import logging
from tempfile import SpooledTemporaryFile
from typing import Optional, cast

from fastapi import UploadFile

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


async def check_books_page(ctx, page_number: int) -> None:
    arq_pool: ArqRedis = ctx["arc_pool"]

    page = await get_books(page_number, PAGE_SIZE)

    object_ids = [book.id for book in page.items]

    cached_files = await CachedFile.objects.filter(object_id__in=object_ids).all()

    cached_files_map = collections.defaultdict(set)
    for cached_file in cached_files:
        cached_files_map[cached_file.object_id].add(cached_file.object_type)

    for book in page.items:
        for file_type in book.available_types:
            if file_type not in cached_files_map[book.id]:
                await arq_pool.enqueue_job("cache_file_by_book_id", book.id, file_type)


async def check_books(ctx: dict, *args, **kwargs) -> None:
    arq_pool: ArqRedis = ctx["arc_pool"]
    books_page = await get_books(1, PAGE_SIZE)

    for page_number in range(books_page.total_pages, 0, -1):
        await arq_pool.enqueue_job("check_books_page", page_number)


async def cache_file(book: Book, file_type) -> Optional[CachedFile]:
    data = await download(book.source.id, book.remote_id, file_type)

    if data is None:
        return None

    response, client, filename = data
    caption = get_caption(book)

    temp_file = UploadFile(filename)
    async for chunk in response.aiter_bytes(2048):
        await temp_file.write(chunk)
    await temp_file.seek(0)

    await response.aclose()
    await client.aclose()

    upload_data = await upload_file(
        cast(SpooledTemporaryFile, temp_file.file), filename, caption
    )

    if upload_data is None:
        return None

    return await CachedFile.objects.create(
        object_id=book.id, object_type=file_type, data=upload_data.data
    )


async def cache_file_by_book_id(
    ctx: dict, book_id: int, file_type: str
) -> Optional[CachedFile]:
    book = await get_book(book_id)

    if file_type not in book.available_types:
        raise FileTypeNotAllowed(f"{file_type} not in {book.available_types}!")

    return await cache_file(book, file_type)
