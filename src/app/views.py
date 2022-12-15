import asyncio
import base64
import csv
from typing import AsyncIterator

from fastapi import APIRouter, Depends, HTTPException, status, Request
from fastapi.responses import StreamingResponse

from starlette.background import BackgroundTask

from arq.connections import ArqRedis
from asyncpg import exceptions

from app.depends import check_token
from app.models import CachedFile as CachedFileDB
from app.serializers import CachedFile, CreateCachedFile
from app.services.cache_updater import cache_file_by_book_id
from app.services.caption_getter import get_caption
from app.services.downloader import get_filename
from app.services.files_client import download_file as download_file_from_cache
from app.services.library_client import get_book
from app.utils import DummyWriter


router = APIRouter(
    prefix="/api/v1", tags=["files"], dependencies=[Depends(check_token)]
)


@router.get("/{object_id}/{object_type}", response_model=CachedFile)
async def get_cached_file(request: Request, object_id: int, object_type: str):
    cached_file = await CachedFileDB.objects.get_or_none(
        object_id=object_id, object_type=object_type
    )

    if not cached_file:
        cached_file = await cache_file_by_book_id(
            {"redis": request.app.state.redis_client}, object_id, object_type
        )

    if not cached_file:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND)

    return cached_file


@router.get("/download/{object_id}/{object_type}")
async def download_cached_file(request: Request, object_id: int, object_type: str):
    cached_file = await CachedFileDB.objects.get_or_none(
        object_id=object_id, object_type=object_type
    )

    if not cached_file:
        cached_file = await cache_file_by_book_id(
            {"redis": request.app.state.redis_client}, object_id, object_type
        )

    if not cached_file:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND)

    cache_data = cached_file.data

    data, filename, book = await asyncio.gather(
        download_file_from_cache(cache_data["chat_id"], cache_data["message_id"]),
        get_filename(object_id, object_type),
        get_book(object_id),
    )

    if data is None:
        await CachedFileDB.objects.filter(id=cached_file.id).delete()
        raise HTTPException(status_code=status.HTTP_204_NO_CONTENT)

    response, client = data

    async def close():
        await response.aclose()
        await client.aclose()

    assert book

    return StreamingResponse(
        response.aiter_bytes(),
        headers={
            "Content-Disposition": f"attachment; filename={filename}",
            "X-Caption-B64": base64.b64encode(get_caption(book).encode("utf-8")).decode(
                "latin-1"
            ),
        },
        background=BackgroundTask(close),
    )


@router.delete("/{object_id}/{object_type}", response_model=CachedFile)
async def delete_cached_file(object_id: int, object_type: str):
    cached_file = await CachedFileDB.objects.get_or_none(
        object_id=object_id, object_type=object_type
    )

    if not cached_file:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND)

    await cached_file.delete()

    return cached_file


@router.post("/", response_model=CachedFile)
async def create_or_update_cached_file(data: CreateCachedFile):
    try:
        return await CachedFileDB.objects.create(**data.dict())
    except exceptions.UniqueViolationError:
        data_dict = data.dict()
        object_id = data_dict.pop("object_id")
        object_type = data_dict.pop("object_type")
        cached_file = await CachedFileDB.objects.get(
            object_id=object_id, object_type=object_type
        )
        cached_file.update_from_dict(data_dict)
        return await cached_file.update()


@router.post("/update_cache")
async def update_cache(request: Request):
    arq_pool: ArqRedis = request.app.state.arq_pool
    await arq_pool.enqueue_job("check_books")

    return "Ok!"


@router.get("/download_dump")
async def download_dump():
    async def get_data() -> AsyncIterator[str]:
        writer = csv.writer(DummyWriter())

        async for c_file in CachedFileDB.objects.iterate():
            yield writer.writerow([c_file.object_id, c_file.object_type, c_file.data])

    return StreamingResponse(
        get_data(),
        headers={
            "Content-Disposition": "attachment; filename=dump.csv",
        },
    )


healthcheck_router = APIRouter(
    tags=["healthcheck"],
)


@healthcheck_router.get("/healthcheck")
async def healthcheck():
    return "Ok!"
