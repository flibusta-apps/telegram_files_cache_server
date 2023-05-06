import sentry_sdk

from core.config import env_config


sentry_sdk.init(
    env_config.SENTRY_DSN,
)
