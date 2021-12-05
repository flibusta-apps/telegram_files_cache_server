from fastapi import APIRouter, Depends, HTTPException, status

from app.depends import check_token
from app.models import CachedFile as CachedFileDB
from app.serializers import CachedFile, CreateCachedFile
from app.services.cache_updater import CacheUpdater


router = APIRouter(
    prefix="/api/v1",
    tags=["files"],
    dependencies=[Depends(check_token)]
)


@router.get("/{object_id}/{object_type}", response_model=CachedFile)
async def get_cached_file(object_id: int, object_type: str):
    cached_file = await CachedFileDB.objects.get_or_none(
        object_id=object_id,
        object_type=object_type
    )

    if not cached_file:
        cached_file = await CacheUpdater.cache_file(object_id, object_type)
    
    if not cached_file:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND)
    
    return cached_file


@router.delete("/{object_id}/{object_type}", response_model=CachedFile)
async def delete_cached_file(object_id: int, object_type: str):
    cached_file = await CachedFileDB.objects.get(
        object_id=object_id,
        object_type=object_type
    )

    if not cached_file:
        raise HTTPException(status_code=status.HTTP_404_NOT_FOUND)

    await cached_file.delete()
    
    return cached_file


@router.post("/", response_model=CachedFile)
async def create_cached_file(data: CreateCachedFile):
    return await CachedFileDB.objects.create(
        **data.dict()
    )


@router.post("/update_cache")
async def update_cache():
    await CacheUpdater.update()
