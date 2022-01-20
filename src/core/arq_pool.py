from arq.connections import create_pool, RedisSettings, ArqRedis

from core.config import env_config


def get_redis_settings() -> RedisSettings:
    return RedisSettings(
        host=env_config.REDIS_HOST,
        port=env_config.REDIS_PORT,
        database=env_config.REDIS_DB,
    )


async def get_arq_pool() -> ArqRedis:
    return await create_pool(get_redis_settings())
