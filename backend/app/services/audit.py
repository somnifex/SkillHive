from typing import Any

from sqlalchemy.orm import Session

from app.models import AuditLog


def write_audit(
    session: Session,
    *,
    action: str,
    resource_type: str,
    actor_user_id: str | None = None,
    resource_id: str | None = None,
    before_data: dict[str, Any] | None = None,
    after_data: dict[str, Any] | None = None,
    ip_address: str | None = None,
    user_agent: str | None = None,
    result: str = "success",
    error_message: str | None = None,
) -> AuditLog:
    log = AuditLog(
        actor_user_id=actor_user_id,
        action=action,
        resource_type=resource_type,
        resource_id=resource_id,
        before_data=before_data,
        after_data=after_data,
        ip_address=ip_address,
        user_agent=user_agent,
        result=result,
        error_message=error_message,
    )
    session.add(log)
    return log
