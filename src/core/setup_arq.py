from app.services.cache_updater import (
    check_books,
    cache_file_by_book_id,
    check_books_page,
)
from core.arq_pool import get_redis_settings, get_arq_pool
from core.db import database


async def startup(ctx):
    if not database.is_connected:
        await database.connect()

    ctx["arc_pool"] = await get_arq_pool()


async def shutdown(ctx):
    if database.is_connected:
        await database.disconnect()


class WorkerSettings:
    functions = [check_books, cache_file_by_book_id, check_books_page]
    on_startup = startup
    on_shutdown = shutdown
    redis_settings = get_redis_settings()
    max_jobs = 4
