from datetime import datetime
from typing import Optional
from tempfile import SpooledTemporaryFile

import httpx
from pydantic import BaseModel

from core.config import env_config


class UploadedFile(BaseModel):
    id: int
    backend: str
    data: dict
    upload_time: datetime


async def upload_file(content: SpooledTemporaryFile, filename: str, caption: str) -> UploadedFile:
    headers = {"Authorization": env_config.FILES_SERVER_API_KEY}

    async with httpx.AsyncClient() as client:
        form = {"caption": caption}
        files = {"file": (filename, content)}

        response = await client.post(
            f"{env_config.FILES_SERVER_URL}/api/v1/files/upload/",
            data=form,
            files=files,
            headers=headers,
            timeout=5 * 60,
        )

        return UploadedFile.parse_obj(response.json())


async def download_file(
    chat_id: int, message_id: int
) -> Optional[tuple[httpx.Response, httpx.AsyncClient]]:
    headers = {"Authorization": env_config.FILES_SERVER_API_KEY}

    client = httpx.AsyncClient(timeout=60)
    request = client.build_request(
        "GET",
        f"{env_config.FILES_SERVER_URL}"
        f"/api/v1/files/download_by_message/{chat_id}/{message_id}",
        headers=headers,
    )

    response = await client.send(request, stream=True)

    if response.status_code != 200:
        return None

    return response, client
