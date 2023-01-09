from fastapi import FastAPI
from fastapi.responses import ORJSONResponse
from prometheus_fastapi_instrumentator import Instrumentator

from app.views import healthcheck_router, router
from core.arq_pool import get_arq_pool
from core.db import database
from core.redis_client import get_client
import core.sentry  # noqa: F401


def start_app() -> FastAPI:
    app = FastAPI(default_response_class=ORJSONResponse)

    app.state.database = database

    app.include_router(router)
    app.include_router(healthcheck_router)

    @app.on_event("startup")
    async def startup() -> None:
        database_ = app.state.database
        if not database_.is_connected:
            await database_.connect()

        app.state.arq_pool = await get_arq_pool()
        app.state.redis_client = get_client()

    @app.on_event("shutdown")
    async def shutdown() -> None:
        database_ = app.state.database
        if database_.is_connected:
            await database_.disconnect()

    Instrumentator().instrument(app).expose(app, include_in_schema=True)

    return app
