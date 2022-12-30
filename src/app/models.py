import ormar

from core.db import metadata, database


class BaseMeta(ormar.ModelMeta):
    metadata = metadata
    database = database


class CachedFile(ormar.Model):
    class Meta(BaseMeta):
        tablename = "cached_files"
        constraints = [
            ormar.UniqueColumns("object_id", "object_type"),
            ormar.UniqueColumns("message_id", "chat_id"),
        ]

    id: int = ormar.Integer(primary_key=True)  # type: ignore
    object_id: int = ormar.Integer(index=True)  # type: ignore
    object_type: str = ormar.String(
        max_length=8, index=True, unique=True
    )  # type: ignore

    message_id: int = ormar.BigInteger(index=True)  # type: ignore
    chat_id: int = ormar.BigInteger()  # type: ignore

    @ormar.property_field
    def data(self) -> dict:
        return {"message_id": self.message_id, "chat_id": self.chat_id}
