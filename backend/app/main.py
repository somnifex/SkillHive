import asyncio
import contextlib
import logging
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from datetime import timedelta

from fastapi import FastAPI, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse, Response
from slowapi import Limiter
from slowapi.errors import RateLimitExceeded
from slowapi.middleware import SlowAPIMiddleware
from slowapi.util import get_remote_address

from app.api.v1.router import router as api_router
from app.core.config import settings
from app.core.exceptions import register_exception_handlers
from app.core.observability import RequestContextMiddleware, configure_logging
from app.db.base import utc_now
from app.db.session import SessionLocal
from app.services.blob_gc import run_blob_gc
from app.services.blob_storage import get_blob_storage
from app.services.sync_trim import trim_expired_rows
from app.services.trash_gc import purge_expired_trash

logger = logging.getLogger("skillhive.maintenance")


async def _maintenance_worker() -> None:
    """Run one bounded, serialized maintenance pass per interval.

    The task owns its own short-lived session per run, so failures never leak
    connections and never block the API event loop beyond one bounded pass.
    Trash purge runs first and commits before trim/GC, preventing a purge and
    mark-and-sweep from observing each other's half-finished transactions.
    """
    while True:
        try:
            await asyncio.to_thread(_maintenance_once)
        except Exception:  # noqa: BLE001 - the background sweep must never crash the app
            logger.exception("maintenance pass failed")
        await asyncio.sleep(max(1, settings.maintenance_interval_seconds))


def _maintenance_once() -> None:
    # Keep the existing savepoint-per-row trash semantics and let it commit
    # before the independent destructive jobs begin.
    with SessionLocal() as session:
        purged = purge_expired_trash(session)

    now = utc_now()
    with SessionLocal() as session:
        trim_report = trim_expired_rows(
            session,
            now=now,
            change_retention=timedelta(days=settings.change_log_retention_days),
            receipt_retention=timedelta(days=settings.receipt_retention_days),
            batch_size=settings.blob_gc_batch_size,
        )
        session.commit()

    with SessionLocal() as session:
        storage = get_blob_storage(session)
        gc_report = run_blob_gc(
            session,
            storage,
            now=now,
            orphan_grace=timedelta(hours=settings.blob_gc_orphan_grace_hours),
            change_retention=timedelta(days=settings.change_log_retention_days),
            receipt_retention=timedelta(days=settings.receipt_retention_days),
            batch_size=settings.blob_gc_batch_size,
        )
        session.commit()

    logger.info(
        "maintenance pass completed purged_trash=%d trimmed_changes=%d "
        "trimmed_receipts=%d gc_candidates=%d gc_deleted=%d",
        purged,
        trim_report.change_log_deleted,
        trim_report.receipts_deleted,
        gc_report.candidate_count,
        gc_report.deleted_count,
    )


@asynccontextmanager
async def lifespan(_app: FastAPI) -> AsyncIterator[None]:
    task = asyncio.create_task(_maintenance_worker(), name="maintenance-worker")
    try:
        yield
    finally:
        task.cancel()
        with contextlib.suppress(asyncio.CancelledError):
            await task


def rate_limit_handler(request: Request, exc: Exception) -> Response:
    del request, exc
    return JSONResponse(
        status_code=429,
        content={
            "error": {
                "code": "RATE_LIMITED",
                "message": "Too many requests. Try again later.",
                "details": None,
            }
        },
    )



def create_app() -> FastAPI:
    # Not gated on first-call: alembic's fileConfig can reconfigure root
    # logging mid-process (e.g. the migration test), so always re-assert the
    # SkillHive handler; configure_logging deduplicates it.
    configure_logging(settings.log_level)
    app = FastAPI(
        title=settings.app_name,
        description="SkillHive team skill management API",
        version="0.1.0",
        docs_url="/docs",
        redoc_url="/redoc",
        lifespan=lifespan,
    )
    # Correlation-ID assignment/echo + structured access logging must run
    # before (outside) the exception handlers so every response — including
    # handler-generated error envelopes — carries the header.
    app.add_middleware(RequestContextMiddleware)
    app.add_middleware(
        CORSMiddleware,
        allow_origins=settings.cors_origins,
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"],
    )
    limiter = Limiter(key_func=get_remote_address, default_limits=["300/minute"])
    app.state.limiter = limiter
    app.add_exception_handler(RateLimitExceeded, rate_limit_handler)
    app.add_middleware(SlowAPIMiddleware)
    register_exception_handlers(app)
    app.include_router(api_router, prefix=settings.api_v1_prefix)
    return app


app = create_app()
