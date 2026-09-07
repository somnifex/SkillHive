from collections.abc import AsyncIterator
from contextlib import asynccontextmanager

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


@asynccontextmanager
async def lifespan(_app: FastAPI) -> AsyncIterator[None]:
    yield


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
