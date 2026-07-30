from typing import Annotated

from fastapi import APIRouter, Cookie, Depends, Request, Response
from sqlalchemy.orm import Session

from app.core.config import settings
from app.core.exceptions import AppError
from app.db.session import get_db
from app.permissions.dependencies import CurrentUser
from app.schemas.auth import (
    ChangePasswordRequest,
    ForgotPasswordRequest,
    LoginRequest,
    RegisterRequest,
    TokenResponse,
)
from app.schemas.common import MessageResponse
from app.schemas.user import UserRead
from app.services.auth import AuthService
from app.services.login_guard import login_guard

router = APIRouter(prefix="/auth", tags=["authentication"])
REFRESH_COOKIE = "skillhive_refresh"


def _client_ip(request: Request) -> str | None:
    return request.client.host if request.client else None


def _set_refresh_cookie(response: Response, token: str) -> None:
    response.set_cookie(
        REFRESH_COOKIE,
        token,
        max_age=settings.refresh_token_expire_days * 86400,
        httponly=True,
        secure=settings.cookie_secure,
        samesite="lax",
        path=f"{settings.api_v1_prefix}/auth",
    )


@router.post("/register", response_model=UserRead, status_code=201)
def register(data: RegisterRequest, session: Annotated[Session, Depends(get_db)]) -> UserRead:
    return UserRead.model_validate(AuthService(session).register(data))


@router.post("/login", response_model=TokenResponse)
def login(
    data: LoginRequest,
    request: Request,
    response: Response,
    session: Annotated[Session, Depends(get_db)],
) -> TokenResponse:
    ip_address = _client_ip(request)
    key = f"{ip_address}:{data.username.strip().lower()}"
    if retry_after := login_guard.check(key):
        raise AppError(
            "LOGIN_RATE_LIMITED",
            "Too many failed login attempts. Try again later.",
            429,
            {"retry_after": retry_after},
        )
    service = AuthService(session)
    try:
        user = service.authenticate(
            data.username,
            data.password,
            ip_address=ip_address,
            user_agent=request.headers.get("user-agent"),
        )
    except AppError:
        login_guard.failure(key)
        raise
    login_guard.success(key)
    access_token, refresh_token = service.issue_tokens(
        user,
        ip_address=ip_address,
        user_agent=request.headers.get("user-agent"),
    )
    _set_refresh_cookie(response, refresh_token)
    return TokenResponse(
        access_token=access_token,
        expires_in=settings.access_token_expire_minutes * 60,
        user=UserRead.model_validate(user),
    )


@router.post("/refresh", response_model=TokenResponse)
def refresh(
    request: Request,
    response: Response,
    session: Annotated[Session, Depends(get_db)],
    refresh_token: Annotated[str | None, Cookie(alias=REFRESH_COOKIE)] = None,
) -> TokenResponse:
    if refresh_token is None:
        raise AppError("MISSING_REFRESH_TOKEN", "Refresh cookie is missing.", 401)
    user, access_token, refresh_token_new = AuthService(session).refresh(
        refresh_token,
        ip_address=_client_ip(request),
        user_agent=request.headers.get("user-agent"),
    )
    _set_refresh_cookie(response, refresh_token_new)
    return TokenResponse(
        access_token=access_token,
        expires_in=settings.access_token_expire_minutes * 60,
        user=UserRead.model_validate(user),
    )


@router.post("/logout", response_model=MessageResponse)
def logout(
    response: Response,
    session: Annotated[Session, Depends(get_db)],
    refresh_token: Annotated[str | None, Cookie(alias=REFRESH_COOKIE)] = None,
) -> MessageResponse:
    AuthService(session).logout(refresh_token)
    response.delete_cookie(REFRESH_COOKIE, path=f"{settings.api_v1_prefix}/auth")
    return MessageResponse(message="Logged out.")


@router.get("/me", response_model=UserRead)
def current_user(user: CurrentUser) -> UserRead:
    return UserRead.model_validate(user)


@router.post("/change-password", response_model=MessageResponse)
def change_password(
    data: ChangePasswordRequest,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> MessageResponse:
    AuthService(session).change_password(user, data.current_password, data.new_password)
    return MessageResponse(message="Password changed. Please sign in again.")


@router.post("/forgot-password", response_model=MessageResponse, status_code=202)
def forgot_password(_data: ForgotPasswordRequest) -> MessageResponse:
    return MessageResponse(
        message="If an active account matches that email, reset instructions will be sent."
    )
