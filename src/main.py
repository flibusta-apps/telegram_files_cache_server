import sentry_sdk

from core.app import start_app
from core.config import env_config


if env_config.SENTRY_DSN:
    sentry_sdk.init(
        dsn=env_config.SENTRY_DSN,
    )

app = start_app()
