from typing import Optional

import httpx

from core.config import env_config


async def download(source_id: int, remote_id: int, file_type: str) -> Optional[tuple[bytes, str]]:
    headers = {"Authorization": env_config.DOWNLOADER_API_KEY}

    async with httpx.AsyncClient() as client:
        response = await client.get(f"{env_config.DOWNLOADER_URL}/download/{source_id}/{remote_id}/{file_type}", headers=headers, timeout=5 * 60)

        if response.status_code != 200:
            return None

        content_disposition = response.headers['Content-Disposition']

        name = content_disposition.replace('attachment; filename=', '')

        return response.content, name
