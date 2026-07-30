from sqlalchemy import or_, select
from sqlalchemy.orm import Session

from app.models import User


class UserRepository:
    def __init__(self, session: Session) -> None:
        self.session = session

    def get(self, user_id: str) -> User | None:
        return self.session.get(User, user_id)

    def by_username_or_email(self, identity: str) -> User | None:
        normalized = identity.strip().lower()
        return self.session.scalar(
            select(User).where(
                or_(User.username == normalized, User.email == normalized),
            )
        )

    def username_exists(self, username: str) -> bool:
        return self.session.scalar(select(User.id).where(User.username == username)) is not None

    def email_exists(self, email: str) -> bool:
        return self.session.scalar(select(User.id).where(User.email == email)) is not None

    def add(self, user: User) -> User:
        self.session.add(user)
        self.session.flush()
        return user
