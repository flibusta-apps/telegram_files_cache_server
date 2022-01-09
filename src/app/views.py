from fastapi import APIRouter, Depends, HTTPException, BackgroundTasks, status

from starlette.responses import Response

from asyncpg import exceptions

from app.depends import check_token
from app.models import CachedFile as CachedFileDB
from app.serializers import CachedFile, CreateCachedFile
from app.services.cache_updater import CacheUpdater
from app.services.downloader import get_filename
from app.services.files_client import download_file as download_file_from_cache


router = APIRouter(
    prefix="/api/v1", tags=["files"], dependencies=[Depends(check_token)]
)


@router.get("/{object_id}/{object_type}", response_model=CachedFile)
async def get_cached_file(object_id: int, object_type: str):
    cached_file = await CachedFileDB.objects.get_or_none(
        object_id=object_id, object_type=object_type
    )

    if not cached_file:
        cached_file = await CacheUpdater.cache_file(object_id, object_type)

    if not cached_file:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND)

    return cached_file


@router.get("/download/{object_id}/{object_type}")
async def download_cached_file(object_id: int, object_type: str):
    cached_file = await CachedFileDB.objects.get_or_none(
        object_id=object_id, object_type=object_type
    )

    if not cached_file:
        cached_file = await CacheUpdater.cache_file(object_id, object_type)

    if not cached_file:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND)

    cache_data = cached_file.data

    data = await download_file_from_cache(
        cache_data["chat_id"], cache_data["message_id"]
    )

    if data is None:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND)

    filename = await get_filename(object_id, object_type)

    return Response(
        data, headers={"Content-Disposition": f"attachment; filename={filename}"}
    )


@router.delete("/{object_id}/{object_type}", response_model=CachedFile)
async def delete_cached_file(object_id: int, object_type: str):
    cached_file = await CachedFileDB.objects.get(
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
async def update_cache(background_tasks: BackgroundTasks):
    background_tasks.add_task(CacheUpdater.update)

    return "Ok!"
