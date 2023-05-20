import logging
from typing import Any

from taskiq import SimpleRetryMiddleware
from taskiq.message import TaskiqMessage
from taskiq.result import TaskiqResult


logger = logging.getLogger("taskiq_middleware")


class FastAPIREtryMiddleware(SimpleRetryMiddleware):
    @staticmethod
    def _is_need_to_remove(to_remove: list[Any], value: Any) -> bool:
        logger.info(f"{type(value)}, {to_remove}")
        return type(value) in to_remove

    async def on_error(
        self, message: TaskiqMessage, result: TaskiqResult[Any], exception: Exception
    ) -> None:
        logger.info(f"{self.broker.custom_dependency_context}")

        types_to_remove = list(self.broker.custom_dependency_context.keys())

        message.args = [
            arg
            for arg in message.args
            if not self._is_need_to_remove(types_to_remove, arg)
        ]
        message.kwargs = {
            key: value
            for key, value in message.kwargs.items()
            if not self._is_need_to_remove(types_to_remove, value)
        }

        return await super().on_error(message, result, exception)
