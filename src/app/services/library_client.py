from typing import Generic, TypeVar
from pydantic import BaseModel
from datetime import date

from core.config import env_config

import httpx


T = TypeVar('T')


class Page(BaseModel, Generic[T]):
    items: list[T]
    total: int
    page: int
    size: int


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


async def get_books(page: int, page_size: int) -> Page[Book]:
    headers = {"Authorization": env_config.LIBRARY_API_KEY}

    async with httpx.AsyncClient() as client:
        response = await client.get(f"{env_config.LIBRARY_URL}/api/v1/books/?page={page}&size={page_size}&is_deleted=false", headers=headers)

        data = response.json()

        page_data = Page[Book].parse_obj(data)
        page_data.items = [Book.parse_obj(item) for item in page_data.items]

        return page_data
