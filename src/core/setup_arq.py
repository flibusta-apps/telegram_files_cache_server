import msgpack

from app.services.cache_updater import (
    cache_file_by_book_id,
    check_books,
    check_books_page,
)
from core.arq_pool import get_arq_pool, get_redis_settings
from core.db import database
from core.redis_client import get_client
import core.sentry  # noqa: F401


async def startup(ctx):
    if not database.is_connected:
        await database.connect()

    ctx["arc_pool"] = await get_arq_pool()
    ctx["redis"] = get_client()


async def shutdown(ctx):
    if database.is_connected:
        await database.disconnect()


class WorkerSettings:
    functions = [check_books, check_books_page, cache_file_by_book_id]
    on_startup = startup
    on_shutdown = shutdown
    redis_settings = get_redis_settings()
    max_jobs = 2
    max_tries = 1
    job_timeout = 10 * 60
    job_serializer = msgpack.packb
    job_deserializer = lambda b: msgpack.unpackb(b, raw=False)  # noqa: E731
