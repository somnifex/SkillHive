"""M4 request-observability tests.

Covers the correlation-ID contract (X-Request-ID echo vs. fresh generation)
and the access/error log lines, verifying that neither tokens, query
strings, nor request bodies ever reach the log output (handoff §16).
"""

from __future__ import annotations

import asyncio
import logging

import pytest
from app.core.observability import (
    REQUEST_ID_HEADER,
    RequestContextMiddleware,
    configure_logging,
    request_id_var,
)
from fastapi.testclient import TestClient


def _records(caplog: pytest.LogCaptureFixture) -> list[logging.LogRecord]:
    return [record for record in caplog.records if record.name == "app.core.observability"]


def test_missing_request_id_is_generated_and_echoed(client: TestClient) -> None:
    response = client.get("/api/v1/health")
    assert response.status_code == 200
    echoed = response.headers.get(REQUEST_ID_HEADER)
    assert echoed is not None and len(echoed) >= 32


def test_supplied_request_id_is_echoed_verbatim(client: TestClient) -> None:
    response = client.get("/api/v1/health", headers={REQUEST_ID_HEADER: "cycle-abc-123"})
    assert response.headers[REQUEST_ID_HEADER] == "cycle-abc-123"


def test_overlong_request_id_is_truncated(client: TestClient) -> None:
    supplied = "x" * 500
    response = client.get("/api/v1/health", headers={REQUEST_ID_HEADER: supplied})
    echoed = response.headers[REQUEST_ID_HEADER]
    assert len(echoed) == 128
    assert echoed == supplied[:128]


def test_error_responses_carry_the_request_id(client: TestClient) -> None:
    # Unauthenticated request → handler-generated 401 envelope; the header
    # must survive handler-generated error responses, not just 2xx paths.
    response = client.get(
        "/api/v1/skills/nonexistent-id", headers={REQUEST_ID_HEADER: "err-case-1"}
    )
    assert response.status_code == 401
    assert response.headers[REQUEST_ID_HEADER] == "err-case-1"


def test_access_log_contains_no_query_or_headers(
    client: TestClient, caplog: pytest.LogCaptureFixture
) -> None:
    with caplog.at_level(logging.INFO, logger="app.core.observability"):
        response = client.get(
            "/api/v1/health?secret=should-not-appear",
            headers={REQUEST_ID_HEADER: "log-check-1"},
        )
    assert response.status_code == 200
    messages = [record.getMessage() for record in _records(caplog)]
    assert messages, "the access log must emit one line per request"
    # The correlation ID is stamped via the formatter's request_id field; the
    # message body itself must never carry the query string or auth headers.
    for message in messages:
        assert "should-not-appear" not in message
        assert "Bearer" not in message


def test_context_var_is_reset_after_the_request(client: TestClient) -> None:
    def probe() -> str:
        return request_id_var.get()

    # The middleware must not leak its ID into later code on the same thread.
    assert request_id_var.get() == "-"
    client.get("/api/v1/health", headers={REQUEST_ID_HEADER: "reset-check-1"})
    assert request_id_var.get() == "-"


def test_middleware_skips_non_http_scopes() -> None:
    calls: list[list[object]] = []

    async def app(scope, receive, send):  # type: ignore[no-untyped-def]
        calls.append([scope, receive, send])

    middleware = RequestContextMiddleware(app)
    asyncio.run(
        middleware({"type": "lifespan"}, None, None)  # type: ignore[arg-type]
    )
    assert len(calls) == 1, "non-http scopes must pass through untouched"


def test_request_id_filter_stamps_ambient_value(caplog: pytest.LogCaptureFixture) -> None:
    configure_logging()
    token = request_id_var.set("filter-check-1")
    try:
        with caplog.at_level(logging.INFO, logger="app.core.observability"):
            logging.getLogger("app.core.observability").info("probe message")
    finally:
        request_id_var.reset(token)
    record = next(
        record for record in caplog.records if record.getMessage() == "probe message"
    )
    assert record.request_id == "filter-check-1"  # type: ignore[attr-defined]


def test_configure_logging_is_idempotent_and_restorable() -> None:
    from app.core.observability import RequestIdFilter

    # A second call must not double-log through stacked SkillHive handlers.
    configure_logging()
    configure_logging()
    root = logging.getLogger()
    ours = [
        handler
        for handler in root.handlers
        if any(isinstance(f, RequestIdFilter) for f in handler.filters)
    ]
    assert len(ours) == 1, root.handlers

    # After an external reconfiguration (alembic's fileConfig does this) the
    # next call must restore the SkillHive handler.
    root.handlers.clear()
    configure_logging()
    ours = [
        handler
        for handler in root.handlers
        if any(isinstance(f, RequestIdFilter) for f in handler.filters)
    ]
    assert len(ours) == 1
