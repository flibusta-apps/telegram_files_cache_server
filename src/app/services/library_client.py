from datetime import date
from typing import Generic, TypeVar, Optional

import httpx
from pydantic import BaseModel

from core.config import env_config


T = TypeVar("T")


class Page(BaseModel, Generic[T]):
    items: list[T]
    total: int
    page: int
    size: int
    total_pages: int


class BookAuthor(BaseModel):
    id: int
    first_name: str
    last_name: str
    middle_name: str


class BookSource(BaseModel):
    id: int
    name: str


class Book(BaseModel):
    id: int
    title: str
    file_type: str
    available_types: list[str]
    source: BookSource
    remote_id: int
    uploaded: date
    authors: list[BookAuthor]


class BookDetail(Book):
    is_deleted: bool


AUTH_HEADERS = {"Authorization": env_config.LIBRARY_API_KEY}


async def get_book(book_id: int, retry: int = 3) -> Optional[BookDetail]:
    if retry == 0:
        return None

    try:
        async with httpx.AsyncClient(timeout=2 * 60) as client:
            response = await client.get(
                f"{env_config.LIBRARY_URL}/api/v1/books/{book_id}", headers=AUTH_HEADERS
            )

            if response.status_code != 200:
                return None

            return BookDetail.parse_obj(response.json())
    except (httpx.ConnectError, httpx.ReadError, httpx.ReadTimeout):
        return await get_book(book_id, retry=retry - 1)


async def get_books(page: int, page_size: int) -> Page[Book]:
    async with httpx.AsyncClient(timeout=5 * 60) as client:
        response = await client.get(
            (
                f"{env_config.LIBRARY_URL}/api/v1/books/"
                f"?page={page}&size={page_size}&is_deleted=false"
            ),
            headers=AUTH_HEADERS,
        )

        data = response.json()

        page_data = Page[Book].parse_obj(data)
        page_data.items = [Book.parse_obj(item) for item in page_data.items]

        return page_data
