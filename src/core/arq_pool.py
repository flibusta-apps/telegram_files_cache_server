import asyncio
from typing import Any

from arq.connections import ArqRedis, RedisSettings, create_pool
from arq.worker import JobExecutionFailed
import msgpack

from core.config import env_config


def default(obj: Any):
    if isinstance(obj, asyncio.TimeoutError):
        return msgpack.ExtType(0, "")
    elif isinstance(obj, JobExecutionFailed):
        return msgpack.ExtType(1, obj.args[0].encode())
    raise TypeError("Unknown type: %r" % (obj,))


def ext_hook(code: int, data: bytes):
    if code == 0:
        return asyncio.TimeoutError()
    elif code == 1:
        return JobExecutionFailed((data.decode()))
    return msgpack.ExtType(code, data)


def job_serializer(d):
    return msgpack.packb(d, default=default, use_bin_type=True)  # noqa: E731


def job_deserializer(b):
    return msgpack.unpackb(b, ext_hook=ext_hook, raw=False)  # noqa: E731


def get_redis_settings() -> RedisSettings:
    return RedisSettings(
        host=env_config.REDIS_HOST,
        port=env_config.REDIS_PORT,
        database=env_config.REDIS_DB,
    )


async def get_arq_pool() -> ArqRedis:
    return await create_pool(
        get_redis_settings(),
        job_serializer=job_serializer,  # type: ignore
        job_deserializer=job_deserializer,  # noqa: E731
    )
