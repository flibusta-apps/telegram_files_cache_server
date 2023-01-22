from base64 import b64decode
from typing import Optional

import httpx
from sentry_sdk import capture_exception

from core.config import env_config


async def download(
    source_id: int, remote_id: int, file_type: str
) -> Optional[tuple[httpx.Response, httpx.AsyncClient, str]]:
    headers = {"Authorization": env_config.DOWNLOADER_API_KEY}

    client = httpx.AsyncClient(timeout=600)
    request = client.build_request(
        "GET",
        f"{env_config.DOWNLOADER_URL}/download/{source_id}/{remote_id}/{file_type}",
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

    name = b64decode(response.headers["x-filename-b64"]).decode()

    return response, client, name


async def get_filename(book_id: int, file_type: str) -> Optional[str]:
    headers = {"Authorization": env_config.DOWNLOADER_API_KEY}

    try:
        async with httpx.AsyncClient() as client:
            response = await client.get(
                f"{env_config.DOWNLOADER_URL}/filename/{book_id}/{file_type}",
                headers=headers,
                timeout=5 * 60,
            )

            if response.status_code != 200:
                return None

            return response.text
    except httpx.HTTPError as e:
        capture_exception(e)
        return None
