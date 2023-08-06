from typing import BinaryIO, Optional

import httpx
from pydantic import BaseModel
from typing_extensions import TypedDict

from core.config import env_config


class Data(TypedDict):
    chat_id: str | int
    message_id: int


class UploadedFile(BaseModel):
    backend: str
    data: Data


async def upload_file(
    content: BinaryIO, content_size: int, filename: str, caption: str
) -> Optional[UploadedFile]:
    headers = {"Authorization": env_config.FILES_SERVER_API_KEY}

    async with httpx.AsyncClient() as client:
        form = {"caption": caption, "file_size": content_size}
        files = {"file": (filename, content)}

        response = await client.post(
            f"{env_config.FILES_SERVER_URL}/api/v1/files/upload/",
            data=form,
            files=files,
            headers=headers,
            timeout=5 * 60,
        )

        if response.status_code != 200:
            return None

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

    try:
        response = await client.send(request, stream=True)
    except httpx.ConnectError:
        await client.aclose()
        return None

    if response.status_code != 200:
        await response.aclose()
        await client.aclose()
        return None

    return response, client
