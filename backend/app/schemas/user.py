from datetime import datetime
from typing import Literal

from pydantic import BaseModel

from app.schemas.common import ORMModel


class UserRead(ORMModel):
    id: str
    username: str
    display_name: str
    email: str
    avatar_url: str | None
    status: str
    is_global_admin: bool
    created_at: datetime
    updated_at: datetime
    last_login_at: datetime | None


class UserSummary(ORMModel):
    id: str
    username: str
    display_name: str
    avatar_url: str | None


class UserStatusUpdate(BaseModel):
    status: Literal["active", "disabled"]
