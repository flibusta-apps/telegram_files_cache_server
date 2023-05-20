from taskiq import SimpleRetryMiddleware
import taskiq_fastapi
from taskiq_redis import ListQueueBroker, RedisAsyncResultBackend

from core.config import REDIS_URL


broker = (
    ListQueueBroker(url=REDIS_URL)
    .with_result_backend(
        RedisAsyncResultBackend(redis_url=REDIS_URL, result_ex_time=5 * 60)
    )
    .with_middlewares(SimpleRetryMiddleware())
)


taskiq_fastapi.init(broker, "main:app")
