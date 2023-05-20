from contextlib import asynccontextmanager

from fastapi import FastAPI
from fastapi.responses import ORJSONResponse

from prometheus_fastapi_instrumentator import Instrumentator
from redis.asyncio import ConnectionPool

from app.views import healthcheck_router, router
from core.config import REDIS_URL
from core.db import database
from core.taskiq_worker import broker


@asynccontextmanager
async def lifespan(app: FastAPI):
    database = app.state.database
    if not database.is_connected:
        await database.connect()

    if not broker.is_worker_process:
        await broker.startup()

    yield

    if database.is_connected:
        await database.disconnect()

    if not broker.is_worker_process:
        await broker.shutdown()

    await app.state.redis_pool.disconnect()


def start_app() -> FastAPI:
    app = FastAPI(default_response_class=ORJSONResponse, lifespan=lifespan)

    app.state.database = database
    app.state.redis_pool = ConnectionPool.from_url(REDIS_URL)

    app.include_router(router)
    app.include_router(healthcheck_router)

    Instrumentator(
        should_ignore_untemplated=True,
        excluded_handlers=["/docs", "/metrics", "/healthcheck"],
    ).instrument(app).expose(app, include_in_schema=True)

    return app
