from fastapi import HTTPException, Request, status

from app.models import CachedFile as CachedFileDB
from app.services.cache_updater import cache_file_by_book_id


async def get_cached_file_or_cache(
    request: Request, object_id: int, object_type: str
) -> CachedFileDB:
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
