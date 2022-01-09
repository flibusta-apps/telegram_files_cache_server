import ormar

from core.db import metadata, database


class BaseMeta(ormar.ModelMeta):
    metadata = metadata
    database = database


class CachedFile(ormar.Model):
    class Meta(BaseMeta):
        tablename = "cached_files"
        constraints = [ormar.UniqueColumns("object_id", "object_type")]

    id: int = ormar.Integer(primary_key=True)  # type: ignore
    object_id: int = ormar.Integer()  # type: ignore
    object_type: str = ormar.String(max_length=8)  # type: ignore
    data: dict = ormar.JSON()  # type: ignore
