from pydantic import BaseSettings


class EnvConfig(BaseSettings):
    API_KEY: str

    POSTGRES_USER: str
    POSTGRES_PASSWORD: str
    POSTGRES_HOST: str
    POSTGRES_PORT: int
    POSTGRES_DB: str

    DOWNLOADER_API_KEY: str
    DOWNLOADER_URL: str

    LIBRARY_API_KEY: str
    LIBRARY_URL: str

    FILES_SERVER_API_KEY: str
    FILES_SERVER_URL: str

    REDIS_HOST: str
    REDIS_PORT: int
    REDIS_DB: int


env_config = EnvConfig()
