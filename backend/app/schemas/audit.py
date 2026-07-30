from datetime import datetime
from typing import Any

from app.schemas.common import ORMModel


class AuditLogRead(ORMModel):
    id: str
    actor_user_id: str | None
    action: str
    resource_type: str
    resource_id: str | None
    before_data: dict[str, Any] | None
    after_data: dict[str, Any] | None
    ip_address: str | None
    user_agent: str | None
    result: str
    error_message: str | None
    created_at: datetime
