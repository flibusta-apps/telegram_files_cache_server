import asyncio
from typing import Any

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


def default(obj: Any):
    if isinstance(obj, asyncio.TimeoutError):
        return msgpack.ExtType(0, "")
    raise TypeError("Unknown type: %r" % (obj,))


def ext_hook(code: int, data: str):
    if code == 0:
        return asyncio.TimeoutError()
    return msgpack.ExtType(code, data)


def job_serializer(d):
    return msgpack.packb(d, default=default, use_bin_type=True)  # noqa: E731


def job_deserializer(b):
    return msgpack.unpackb(b, ext_hook=ext_hook, raw=False)  # noqa: E731


class WorkerSettings:
    functions = [check_books, check_books_page, cache_file_by_book_id]
    on_startup = startup
    on_shutdown = shutdown
    redis_settings = get_redis_settings()
    max_jobs = 2
    max_tries = 1
    job_timeout = 10 * 60
    expires_extra_ms = 7 * 24 * 60 * 1000
    job_serializer = job_serializer
    job_deserializer = job_deserializer
