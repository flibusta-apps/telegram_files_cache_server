from fastapi import FastAPI
from fastapi.responses import ORJSONResponse

from prometheus_fastapi_instrumentator import Instrumentator
from redis.asyncio import ConnectionPool

from app.views import healthcheck_router, router
from core.config import REDIS_URL
from core.db import database
from core.taskiq_worker import broker


def start_app() -> FastAPI:
    app = FastAPI(default_response_class=ORJSONResponse)

    app.state.redis_pool = ConnectionPool.from_url(REDIS_URL)

    app.include_router(router)
    app.include_router(healthcheck_router)

    @app.on_event("startup")
    async def app_startup():
        if not database.is_connected:
            await database.connect()

        if not broker.is_worker_process:
            await broker.startup()

    @app.on_event("shutdown")
    async def app_shutdown():
        if database.is_connected:
            await database.disconnect()

        if not broker.is_worker_process:
            await broker.shutdown()

        await app.state.redis_pool.disconnect()

    Instrumentator(
        should_ignore_untemplated=True,
        excluded_handlers=["/docs", "/metrics", "/healthcheck"],
    ).instrument(app).expose(app, include_in_schema=True)

    return app
