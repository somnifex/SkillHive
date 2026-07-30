from typing import Annotated

from fastapi import Depends
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.core.security import decode_token
from app.db.session import get_db
from app.models import User
from app.repositories.users import UserRepository

bearer = HTTPBearer(auto_error=False)

DBSession = Annotated[Session, Depends(get_db)]


def get_current_user(
    session: DBSession,
    credentials: Annotated[HTTPAuthorizationCredentials | None, Depends(bearer)],
) -> User:
    if credentials is None:
        raise AppError("NOT_AUTHENTICATED", "Authentication is required.", 401)
    try:
        payload = decode_token(credentials.credentials, "access")
    except ValueError as exc:
        raise AppError("INVALID_ACCESS_TOKEN", "Access token is invalid or expired.", 401) from exc
    user = UserRepository(session).get(str(payload["sub"]))
    if user is None or user.status != "active":
        raise AppError("USER_DISABLED", "This account is unavailable.", 403)
    return user


CurrentUser = Annotated[User, Depends(get_current_user)]


def require_global_admin(user: CurrentUser) -> User:
    if not user.is_global_admin:
        raise AppError(
            "PERMISSION_DENIED",
            "Global administrator permission is required.",
            403,
        )
    return user


GlobalAdmin = Annotated[User, Depends(require_global_admin)]
