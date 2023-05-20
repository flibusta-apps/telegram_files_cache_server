from fastapi import HTTPException, Request, Security, status

from redis.asyncio import ConnectionPool
from taskiq import TaskiqDepends

from core.auth import default_security
from core.config import env_config


async def check_token(api_key: str = Security(default_security)):
    if api_key != env_config.API_KEY:
        raise HTTPException(
            status_code=status.HTTP_403_FORBIDDEN, detail="Wrong api key!"
        )


def get_redis_pool(request: Request = TaskiqDepends()) -> ConnectionPool:
    return request.app.state.redis_pool
