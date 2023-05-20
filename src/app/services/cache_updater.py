import collections
from io import BytesIO
from tempfile import SpooledTemporaryFile
from typing import Optional, cast

from fastapi import UploadFile

import httpx
from redis.asyncio import ConnectionPool, Redis
from taskiq import TaskiqDepends

from app.depends import get_redis_pool
from app.models import CachedFile
from app.services.caption_getter import get_caption
from app.services.downloader import download
from app.services.files_client import upload_file
from app.services.library_client import Book, get_book, get_books, get_last_book_id
from core.taskiq_worker import broker


PAGE_SIZE = 100


class Retry(Exception):
    pass


class FileTypeNotAllowed(Exception):
    def __init__(self, message: str) -> None:
        super().__init__(message)


@broker.task
async def check_books_page(page_number: int) -> bool:
    page = await get_books(page_number, PAGE_SIZE)

    object_ids = [book.id for book in page.items]

    cached_files = await CachedFile.objects.filter(object_id__in=object_ids).all()

    cached_files_map = collections.defaultdict(set)
    for cached_file in cached_files:
        cached_files_map[cached_file.object_id].add(cached_file.object_type)

    for book in page.items:
        for file_type in book.available_types:
            if file_type not in cached_files_map[book.id]:
                await cache_file_by_book_id.kiq(
                    book_id=book.id,
                    file_type=file_type,
                    by_request=False,
                )

    return True


@broker.task
async def check_books(*args, **kwargs) -> bool:  # NOSONAR
    last_book_id = await get_last_book_id()

    for page_number in range(0, last_book_id // 100 + 1):
        await check_books_page.kiq(page_number)

    return True


async def cache_file(book: Book, file_type: str) -> Optional[CachedFile]:
    if await CachedFile.objects.filter(
        object_id=book.id, object_type=file_type
    ).exists():
        return
    try:
        data = await download(book.source.id, book.remote_id, file_type)
    except httpx.HTTPError:
        data = None

    if data is None:
        raise Retry

    response, client, filename = data
    caption = get_caption(book)

    temp_file = UploadFile(BytesIO(), filename=filename)
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
        object_id=book.id,
        object_type=file_type,
        message_id=upload_data.data["message_id"],
        chat_id=upload_data.data["chat_id"],
    )


@broker.task(retry_on_error=True)
async def cache_file_by_book_id(
    book_id: int,
    file_type: str,
    by_request: bool = True,
    redis_pool: ConnectionPool = TaskiqDepends(get_redis_pool),
) -> Optional[CachedFile]:
    book = await get_book(book_id, 3)

    if book is None:
        if by_request:
            return None
        raise Retry

    if file_type not in book.available_types:
        return None

    async with Redis(connection_pool=redis_pool) as redis_client:
        lock = redis_client.lock(
            f"{book_id}_{file_type}", blocking_timeout=5, thread_local=False
        )

        try:
            async with lock:
                result = await cache_file(book, file_type)
        except Exception as e:
            if by_request:
                return None
            raise Retry from e

    if by_request:
        return result
    return None
