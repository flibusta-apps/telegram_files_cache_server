from redis import asyncio as aioredis

from core.config import env_config


def get_client() -> aioredis.Redis:
    return aioredis.Redis(
        host=env_config.REDIS_HOST, port=env_config.REDIS_PORT, db=env_config.REDIS_DB
    )
