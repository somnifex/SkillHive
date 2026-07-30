from datetime import UTC, datetime, timedelta

from sqlalchemy import select, update
from sqlalchemy.orm import Session

from app.core.config import settings
from app.core.exceptions import AppError
from app.core.security import create_token, decode_token, hash_password, verify_password
from app.db.base import utc_now
from app.models import TokenSession, User
from app.repositories.users import UserRepository
from app.schemas.auth import RegisterRequest
from app.services.audit import write_audit
from app.services.templates import ensure_default_template


class AuthService:
    def __init__(self, session: Session) -> None:
        self.session = session
        self.users = UserRepository(session)

    def register(self, data: RegisterRequest) -> User:
        username = data.username.strip().lower()
        email = str(data.email).lower()
        if self.users.username_exists(username):
            raise AppError("USERNAME_TAKEN", "Username is already in use.", 409)
        if self.users.email_exists(email):
            raise AppError("EMAIL_TAKEN", "Email is already in use.", 409)
        user = self.users.add(
            User(
                username=username,
                display_name=data.display_name.strip(),
                email=email,
                password_hash=hash_password(data.password),
                status="active",
            )
        )
        ensure_default_template(self.session, user)
        write_audit(
            self.session,
            actor_user_id=user.id,
            action="user.registered",
            resource_type="user",
            resource_id=user.id,
            after_data={"username": user.username, "email": user.email},
        )
        self.session.commit()
        return user

    def authenticate(
        self,
        identity: str,
        password: str,
        *,
        ip_address: str | None,
        user_agent: str | None,
    ) -> User:
        user = self.users.by_username_or_email(identity)
        if user is None or not verify_password(password, user.password_hash):
            write_audit(
                self.session,
                action="auth.login_failed",
                resource_type="user",
                resource_id=None,
                after_data={"identity": identity[:100]},
                ip_address=ip_address,
                user_agent=user_agent,
                result="failure",
                error_message="Invalid credentials",
            )
            self.session.commit()
            raise AppError("INVALID_CREDENTIALS", "Invalid username or password.", 401)
        if user.status != "active":
            write_audit(
                self.session,
                actor_user_id=user.id,
                action="auth.login_failed",
                resource_type="user",
                resource_id=user.id,
                ip_address=ip_address,
                user_agent=user_agent,
                result="failure",
                error_message="User disabled",
            )
            self.session.commit()
            raise AppError("USER_DISABLED", "This account is disabled.", 403)
        user.last_login_at = utc_now()
        write_audit(
            self.session,
            actor_user_id=user.id,
            action="auth.login_success",
            resource_type="user",
            resource_id=user.id,
            ip_address=ip_address,
            user_agent=user_agent,
        )
        self.session.commit()
        return user

    def issue_tokens(
        self,
        user: User,
        *,
        ip_address: str | None,
        user_agent: str | None,
    ) -> tuple[str, str]:
        access_token, _, _ = create_token(
            user.id,
            "access",
            timedelta(minutes=settings.access_token_expire_minutes),
        )
        refresh_token, refresh_jti, expires_at = create_token(
            user.id,
            "refresh",
            timedelta(days=settings.refresh_token_expire_days),
        )
        self.session.add(
            TokenSession(
                user_id=user.id,
                refresh_jti=refresh_jti,
                expires_at=expires_at,
                ip_address=ip_address,
                user_agent=user_agent,
            )
        )
        self.session.commit()
        return access_token, refresh_token

    def refresh(
        self,
        refresh_token: str,
        *,
        ip_address: str | None,
        user_agent: str | None,
    ) -> tuple[User, str, str]:
        try:
            payload = decode_token(refresh_token, "refresh")
        except ValueError as exc:
            raise AppError("INVALID_REFRESH_TOKEN", "Refresh token is invalid.", 401) from exc
        token_session = self.session.scalar(
            select(TokenSession).where(TokenSession.refresh_jti == payload["jti"])
        )
        expires_at = token_session.expires_at if token_session is not None else None
        if (
            token_session is None
            or token_session.revoked_at is not None
            or expires_at is None
            or _as_utc(expires_at) <= datetime.now(UTC)
        ):
            raise AppError("SESSION_REVOKED", "This session is no longer valid.", 401)
        user = self.users.get(str(payload["sub"]))
        if user is None or user.status != "active":
            raise AppError("USER_DISABLED", "This account is unavailable.", 403)
        token_session.revoked_at = utc_now()
        access_token, refresh_token_new = self.issue_tokens(
            user,
            ip_address=ip_address,
            user_agent=user_agent,
        )
        return user, access_token, refresh_token_new

    def logout(self, refresh_token: str | None) -> None:
        if refresh_token:
            try:
                payload = decode_token(refresh_token, "refresh")
            except ValueError:
                payload = {}
            if jti := payload.get("jti"):
                self.session.execute(
                    update(TokenSession)
                    .where(TokenSession.refresh_jti == str(jti))
                    .values(revoked_at=utc_now())
                )
                self.session.commit()

    def change_password(self, user: User, current_password: str, new_password: str) -> None:
        if not verify_password(current_password, user.password_hash):
            raise AppError("INVALID_PASSWORD", "Current password is incorrect.", 400)
        if verify_password(new_password, user.password_hash):
            raise AppError("PASSWORD_REUSED", "New password must be different.", 400)
        user.password_hash = hash_password(new_password)
        self.session.execute(
            update(TokenSession)
            .where(TokenSession.user_id == user.id, TokenSession.revoked_at.is_(None))
            .values(revoked_at=utc_now())
        )
        write_audit(
            self.session,
            actor_user_id=user.id,
            action="user.password_changed",
            resource_type="user",
            resource_id=user.id,
        )
        self.session.commit()


def _as_utc(value: datetime) -> datetime:
    return value.replace(tzinfo=UTC) if value.tzinfo is None else value.astimezone(UTC)
