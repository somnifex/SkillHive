"""Request observability: correlation IDs and structured access logging (M4).

Every HTTP request carries a correlation ID: the caller's ``X-Request-ID``
header when supplied (desktop sync cycles send their cycle ID), otherwise a
freshly generated UUID. The ID is echoed in the response header, stamped on
every log record through a context variable, and included in the access log
so a client-reported failure maps to exactly one server-side request.

The access log records method, path, status, duration, and the correlation
ID only — never tokens, cookies, query strings, or request/response bodies
(handoff §16: do not log secrets or sensitive Skill content).
"""

from __future__ import annotations

import logging
import time
import uuid
from contextvars import ContextVar

from starlette.datastructures import Headers, MutableHeaders
from starlette.types import ASGIApp, Message, Receive, Scope, Send

REQUEST_ID_HEADER = "X-Request-ID"
REQUEST_ID_MAX_LENGTH = 128

logger = logging.getLogger(__name__)

request_id_var: ContextVar[str] = ContextVar("request_id", default="-")


def new_request_id() -> str:
    return uuid.uuid4().hex


class RequestIdFilter(logging.Filter):
    """Stamps the ambient correlation ID onto every record passing through."""

    def filter(self, record: logging.LogRecord) -> bool:
        record.request_id = request_id_var.get()
        return True


_configured = False


def configure_logging(level: str = "INFO") -> None:
    """Install the SkillHive log handler once (idempotent).

    Attaches a stream handler with the correlation-ID formatter to the root
    logger so ``app.*`` records (and anything else propagating) carry
    ``request_id=...``. uvicorn keeps its own handlers; ours adds the
    correlation context on top.

    Re-runnable by design: alembic's ``fileConfig`` (migrations/env.py)
    reconfigures root logging mid-process with ``disable_existing_loggers``
    semantics, which can drop the handler installed earlier in the process.
    Calling this again restores the SkillHive handler. The handler is
    deduplicated so repeated calls never double-log.
    """
    global _configured
    root = logging.getLogger()
    handler = logging.StreamHandler()
    handler.setFormatter(
        logging.Formatter(
            "%(asctime)s %(levelname)s [%(name)s] request_id=%(request_id)s %(message)s"
        )
    )
    handler.addFilter(RequestIdFilter())
    _remove_stale_handlers(root)
    root.addHandler(handler)
    root.setLevel(level.upper())
    _configured = True


def _remove_stale_handlers(root: logging.Logger) -> None:
    """Drop prior SkillHive handlers (identified by our filter) to avoid doubles."""
    for existing in list(root.handlers):
        if any(isinstance(f, RequestIdFilter) for f in existing.filters):
            root.removeHandler(existing)


class RequestContextMiddleware:
    """Pure-ASGI middleware assigning and echoing the request correlation ID.

    The ID is taken from the client's ``X-Request-ID`` header (capped to a
    sane length) or generated fresh, set as a context var for the duration
    of the request, echoed in the response header, and included in the
    access log line. Handler-generated error responses (AppError, 4xx/5xx
    envelopes) flow back through this layer and get the header too.
    """

    def __init__(self, app: ASGIApp) -> None:
        self.app = app

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self.app(scope, receive, send)
            return

        headers = Headers(scope=scope)
        supplied = (headers.get(REQUEST_ID_HEADER) or "").strip()[:REQUEST_ID_MAX_LENGTH]
        request_id = supplied or new_request_id()
        token = request_id_var.set(request_id)
        started = time.perf_counter()
        status = 0

        async def send_with_request_id(message: Message) -> None:
            nonlocal status
            if message["type"] == "http.response.start":
                status = message["status"]
                MutableHeaders(scope=message).append(REQUEST_ID_HEADER, request_id)
            await send(message)

        try:
            await self.app(scope, receive, send_with_request_id)
        except Exception:
            # Unhandled crash: still correlated, then re-raised for Starlette's
            # outermost server-error handler.
            logger.exception(
                "http_request_crashed method=%s path=%s", scope["method"], scope["path"]
            )
            raise
        else:
            duration_ms = (time.perf_counter() - started) * 1000
            logger.info(
                "http_request method=%s path=%s status=%s duration_ms=%.1f",
                scope["method"],
                scope["path"],
                status,
                duration_ms,
            )
        finally:
            request_id_var.reset(token)
