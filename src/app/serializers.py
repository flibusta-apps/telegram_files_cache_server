from pydantic import BaseModel, constr


class CachedFile(BaseModel):
    id: int
    object_id: int
    object_type: str
    data: dict


class CreateCachedFile(BaseModel):
    object_id: int
    object_type: constr(max_length=8)  # type: ignore
    data: dict
