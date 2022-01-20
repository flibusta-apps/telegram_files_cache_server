from fastapi import FastAPI

from app.views import router
from core.arq_pool import get_arq_pool
from core.db import database


def start_app() -> FastAPI:
    app = FastAPI()

    app.state.database = database

    app.include_router(router)

    @app.on_event("startup")
    async def startup() -> None:
        database_ = app.state.database
        if not database_.is_connected:
            await database_.connect()

        app.state.arq_pool = await get_arq_pool()

    @app.on_event("shutdown")
    async def shutdown() -> None:
        database_ = app.state.database
        if database_.is_connected:
            await database_.disconnect()

    return app
