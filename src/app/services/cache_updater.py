import collections
from datetime import timedelta
import logging
from tempfile import SpooledTemporaryFile
from typing import Optional, cast

from fastapi import UploadFile

import aioredis
from aioredis.exceptions import LockError
from arq.connections import ArqRedis
from arq.worker import Retry
import httpx

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
                await arq_pool.enqueue_job(
                    "cache_file_by_book_id", book.id, file_type, by_request=False
                )


async def check_books(ctx: dict, *args, **kwargs) -> None:  # NOSONAR
    arq_pool: ArqRedis = ctx["arc_pool"]
    try:
        books_page = await get_books(1, PAGE_SIZE)
    except httpx.ConnectError:
        raise Retry(defer=15)

    for i, page_number in enumerate(range(books_page.total_pages, 0, -1)):
        await arq_pool.enqueue_job("check_books_page", page_number, _defer_by=2 * i)


async def cache_file(book: Book, file_type: str) -> Optional[CachedFile]:
    if await CachedFile.objects.filter(
        object_id=book.id, object_type=file_type
    ).exists():
        return

    retry_exc = Retry(defer=timedelta(minutes=15))

    try:
        data = await download(book.source.id, book.remote_id, file_type)
    except httpx.TimeoutException:
        raise retry_exc

    if data is None:
        raise retry_exc

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
    ctx: dict,  # NOSONAR
    book_id: int,
    file_type: str,
    by_request: bool = True,
) -> Optional[CachedFile]:
    r_client: aioredis.Redis = ctx["redis"]

    try:
        book = await get_book(book_id)
    except httpx.ConnectError:
        if by_request:
            return None
        raise Retry(defer=15)

    if file_type not in book.available_types:
        raise FileTypeNotAllowed(f"{file_type} not in {book.available_types}!")

    lock = r_client.lock(f"{book_id}_{file_type}", blocking_timeout=5)

    try:
        try:
            async with lock:
                return await cache_file(book, file_type)
        except LockError:
            raise Retry(defer=timedelta(minutes=15))
    except Retry as e:
        if by_request:
            return None
        raise e
