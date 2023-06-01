from inspect import signature
from typing import Any

from taskiq import SimpleRetryMiddleware
from taskiq.message import TaskiqMessage
from taskiq.result import TaskiqResult
from taskiq_dependencies.dependency import Dependency


class FastAPIRetryMiddleware(SimpleRetryMiddleware):
    @staticmethod
    def _remove_depends(
        task_func: Any, message_kwargs: dict[str, Any]
    ) -> dict[str, Any]:
        sig = signature(task_func)

        keys_to_remove = []

        for key in message_kwargs.keys():
            param = sig.parameters.get(key, None)

            if param is None:
                continue

            if isinstance(param.default, Dependency):
                keys_to_remove.append(key)

        for key in keys_to_remove:
            message_kwargs.pop(key)

        return message_kwargs

    async def on_error(
        self, message: TaskiqMessage, result: TaskiqResult[Any], exception: Exception
    ) -> None:
        task_func = self.broker.available_tasks[message.task_name].original_func

        message.kwargs = self._remove_depends(task_func, message.kwargs)

        return await super().on_error(message, result, exception)
