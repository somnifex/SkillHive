from dataclasses import dataclass
from datetime import UTC, datetime, timedelta

from app.core.config import settings


@dataclass
class LoginState:
    failures: int = 0
    locked_until: datetime | None = None


class LoginGuard:
    def __init__(self) -> None:
        self._states: dict[str, LoginState] = {}

    def check(self, key: str) -> int | None:
        state = self._states.get(key)
        if state is None or state.locked_until is None:
            return None
        now = datetime.now(UTC)
        if state.locked_until <= now:
            self._states.pop(key, None)
            return None
        return max(1, int((state.locked_until - now).total_seconds()))

    def failure(self, key: str) -> None:
        state = self._states.setdefault(key, LoginState())
        state.failures += 1
        if state.failures >= settings.login_max_attempts:
            state.locked_until = datetime.now(UTC) + timedelta(
                minutes=settings.login_lockout_minutes
            )

    def success(self, key: str) -> None:
        self._states.pop(key, None)

    def clear(self) -> None:
        self._states.clear()


login_guard = LoginGuard()
