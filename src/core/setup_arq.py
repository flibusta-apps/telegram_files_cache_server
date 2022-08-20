import aioredis

from app.services.cache_updater import (
    check_books,
    cache_file_by_book_id,
    check_books_page,
)
from core.arq_pool import get_redis_settings, get_arq_pool
from core.config import env_config
from core.db import database
import core.sentry  # noqa: F401


async def startup(ctx):
    if not database.is_connected:
        await database.connect()

    ctx["arc_pool"] = await get_arq_pool()
    ctx["redis"] = aioredis.Redis(
        host=env_config.REDIS_HOST, port=env_config.REDIS_PORT, db=env_config.REDIS_DB
    )


async def shutdown(ctx):
    if database.is_connected:
        await database.disconnect()


class WorkerSettings:
    functions = [check_books, check_books_page, cache_file_by_book_id]
    on_startup = startup
    on_shutdown = shutdown
    redis_settings = get_redis_settings()
    max_jobs = 2
    max_tries = 3
    job_timeout = 10 * 60
