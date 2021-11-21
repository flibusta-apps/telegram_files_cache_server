from fastapi import FastAPI

from core.db import database
from app.views import router


def start_app() -> FastAPI:
    app = FastAPI()

    app.state.database = database

    app.include_router(router)

    @app.on_event('startup')
    async def startup() -> None:
        database_ = app.state.database
        if not database_.is_connected:
            await database_.connect()

    @app.on_event('shutdown')
    async def shutdown() -> None:
        database_ = app.state.database
        if database_.is_connected:
            await database_.disconnect()

    return app
