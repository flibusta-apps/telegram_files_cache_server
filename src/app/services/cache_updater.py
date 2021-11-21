import asyncio

from app.services.library_client import get_books, Book
from app.services.downloader import download
from app.services.files_uploader import upload_file
from app.models import CachedFile


PAGE_SIZE = 50


class CacheUpdater:
    def __init__(self):
        self.queue = asyncio.Queue(maxsize=10)
        self.all_books_checked = False

    async def _check_book(self, book: Book):
        for file_type in book.available_types:
            cached_file = await CachedFile.objects.get_or_none(
                object_id=book.id, object_type=file_type
            )

            if cached_file is None:
                await self.queue.put((book, file_type))

    async def _start_producer(self):
        books_page = await get_books(1, 1)

        page_count = books_page.total // PAGE_SIZE
        if books_page.total % PAGE_SIZE != 0:
            page_count += 1

        for page_number in range(1, page_count + 1):
            page = await get_books(page_number, PAGE_SIZE)

            for book in page.items:
                await self._check_book(book)
        
        self.all_books_checked = True

    async def _start_worker(self):
        while not self.all_books_checked or not self.queue.empty():
            try:
                task = self.queue.get_nowait()
                book: Book = task[0]
                file_type: str = task[1]
            except asyncio.QueueEmpty:
                await asyncio.sleep(0.1)
                continue
            
            data = await download(book.source.id, book.remote_id, file_type)

            if data is None:
                return

            content, filename = data

            upload_data = await upload_file(content, filename, 'Test')

            await CachedFile.objects.create(
                object_id=book.id,
                object_type=file_type,
                data=upload_data.data
            )

    async def _update(self):
        await asyncio.gather(
            self._start_producer(),
            self._start_worker(),
            self._start_worker()
        )

    @classmethod
    async def update(cls):
        updater = cls()
        return await updater._update()
