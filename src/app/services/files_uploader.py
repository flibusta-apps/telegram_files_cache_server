from datetime import datetime

import httpx
from pydantic import BaseModel

from core.config import env_config


class UploadedFile(BaseModel):
    id: int
    backend: str
    data: dict
    upload_time: datetime


async def upload_file(content: bytes, filename: str, caption: str) -> UploadedFile:
    headers = {"Authorization": env_config.FILES_SERVER_API_KEY}

    async with httpx.AsyncClient() as client:
        form = {'caption': caption}
        files = {'file': (filename, content)}

        response = await client.post(f"{env_config.FILES_SERVER_URL}/api/v1/files/upload/", data=form, files=files, headers=headers, timeout=5 * 60)

        return UploadedFile.parse_obj(response.json())
