from datetime import datetime
from typing import Annotated

from fastapi import APIRouter, Depends, Query, Response
from sqlalchemy.orm import Session

from app.db.session import get_db
from app.permissions.dependencies import GlobalAdmin
from app.schemas.audit import AuditLogRead
from app.schemas.common import Page
from app.schemas.group import AdminGroupStatusUpdate, GroupRead, MemberRead, MemberRoleUpdate
from app.schemas.skill import (
    GlobalSkillCreate,
    GlobalSkillUpdate,
    GroupSkillGrantRead,
    PublishSkillRequest,
    SkillRead,
    SkillVersionCreate,
    SkillVersionRead,
)
from app.schemas.system_settings import (
    AdminPasswordReset,
    AdminUserCreate,
    SystemSettingsRead,
    SystemSettingsUpdate,
)
from app.schemas.user import UserRead, UserStatusUpdate
from app.services.admin import AdminService
from app.services.global_skills import GlobalSkillService
from app.services.groups import GroupService
from app.services.system_settings import SystemSettingsService

router = APIRouter(prefix="/admin", tags=["administration"])


@router.get("/users", response_model=Page[UserRead])
def list_users(
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
    page: Annotated[int, Query(ge=1)] = 1,
    page_size: Annotated[int, Query(ge=1, le=100)] = 20,
    query: Annotated[str | None, Query(max_length=120)] = None,
    status: str | None = None,
) -> Page[UserRead]:
    return AdminService(session, admin).users(
        page=page,
        page_size=page_size,
        query=query,
        status=status,
    )


@router.patch("/users/{user_id}/status", response_model=UserRead)
def update_user_status(
    user_id: str,
    data: UserStatusUpdate,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> UserRead:
    return AdminService(session, admin).set_user_status(user_id, data.status)


@router.post("/users", response_model=UserRead, status_code=201)
def admin_create_user(
    data: AdminUserCreate,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> UserRead:
    return AdminService(session, admin).create_user(data)


@router.post("/users/{user_id}/reset-password", status_code=204)
def admin_reset_password(
    user_id: str,
    data: AdminPasswordReset,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> Response:
    AdminService(session, admin).reset_password(user_id, data.new_password)
    return Response(status_code=204)


@router.delete("/users/{user_id}", status_code=204)
def admin_delete_user(
    user_id: str,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> Response:
    AdminService(session, admin).delete_user(user_id)
    return Response(status_code=204)


@router.get("/groups/tree", response_model=list[GroupRead])
def admin_group_tree(
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> list[GroupRead]:
    return GroupService(session, admin).tree()


@router.get("/system/settings", response_model=SystemSettingsRead)
def get_system_settings(
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> SystemSettingsRead:
    return SystemSettingsService(session, admin).read()


@router.patch("/system/settings", response_model=SystemSettingsRead)
def update_system_settings(
    data: SystemSettingsUpdate,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> SystemSettingsRead:
    return SystemSettingsService(session, admin).update(data)


@router.get("/groups", response_model=Page[GroupRead])
def list_all_groups(
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
    page: Annotated[int, Query(ge=1)] = 1,
    page_size: Annotated[int, Query(ge=1, le=100)] = 20,
    query: Annotated[str | None, Query(max_length=120)] = None,
    status: str | None = None,
) -> Page[GroupRead]:
    return AdminService(session, admin).groups(
        page=page,
        page_size=page_size,
        query=query,
        status=status,
    )


@router.get("/groups/{group_id}/members", response_model=list[MemberRead])
def list_all_group_members(
    group_id: str,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> list[MemberRead]:
    return AdminService(session, admin).group_members(group_id)


@router.patch("/groups/{group_id}/status", response_model=GroupRead)
def update_group_status(
    group_id: str,
    data: AdminGroupStatusUpdate,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> GroupRead:
    return AdminService(session, admin).set_group_status(group_id, data.status)


@router.patch("/groups/{group_id}/members/{user_id}", response_model=MemberRead)
def admin_update_group_role(
    group_id: str,
    user_id: str,
    data: MemberRoleUpdate,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> MemberRead:
    return AdminService(session, admin).set_group_role(group_id, user_id, data.role)


@router.get("/skills", response_model=Page[SkillRead])
def list_global_skills(
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
    page: Annotated[int, Query(ge=1)] = 1,
    page_size: Annotated[int, Query(ge=1, le=100)] = 20,
    query: Annotated[str | None, Query(max_length=120)] = None,
    status: str | None = None,
) -> Page[SkillRead]:
    return GlobalSkillService(session, admin).list_page(
        page=page,
        page_size=page_size,
        query=query,
        status=status,
    )


@router.post("/skills", response_model=SkillRead, status_code=201)
def create_global_skill(
    data: GlobalSkillCreate,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return GlobalSkillService(session, admin).create(data)


@router.get("/skills/{skill_id}", response_model=SkillRead)
def get_global_skill(
    skill_id: str,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return GlobalSkillService(session, admin).get(skill_id)


@router.patch("/skills/{skill_id}", response_model=SkillRead)
def update_global_skill(
    skill_id: str,
    data: GlobalSkillUpdate,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return GlobalSkillService(session, admin).update(skill_id, data)


@router.get("/skills/{skill_id}/versions", response_model=list[SkillVersionRead])
def global_skill_versions(
    skill_id: str,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> list[SkillVersionRead]:
    return GlobalSkillService(session, admin).versions(skill_id)


@router.post("/skills/{skill_id}/versions", response_model=SkillVersionRead, status_code=201)
def create_global_skill_version(
    skill_id: str,
    data: SkillVersionCreate,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> SkillVersionRead:
    return GlobalSkillService(session, admin).create_version(skill_id, data)


@router.post("/skills/{skill_id}/publish", response_model=SkillRead)
def publish_global_skill(
    skill_id: str,
    data: PublishSkillRequest,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return GlobalSkillService(session, admin).publish(skill_id, data.version_id)


@router.post("/skills/{skill_id}/disable", response_model=SkillRead)
def disable_global_skill(
    skill_id: str,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return GlobalSkillService(session, admin).set_status(skill_id, "disabled")


@router.post("/skills/{skill_id}/archive", response_model=SkillRead)
def archive_global_skill(
    skill_id: str,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return GlobalSkillService(session, admin).set_status(skill_id, "archived")


@router.get("/skills/{skill_id}/grants", response_model=list[GroupSkillGrantRead])
def global_skill_grants(
    skill_id: str,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> list[GroupSkillGrantRead]:
    return GlobalSkillService(session, admin).grants(skill_id)


@router.delete("/skills/{skill_id}/grants/{group_id}", status_code=204)
def revoke_global_skill_grant(
    skill_id: str,
    group_id: str,
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
) -> Response:
    GlobalSkillService(session, admin).revoke_grant(skill_id, group_id)
    return Response(status_code=204)


@router.get("/audit-logs", response_model=Page[AuditLogRead])
def list_audit_logs(
    admin: GlobalAdmin,
    session: Annotated[Session, Depends(get_db)],
    page: Annotated[int, Query(ge=1)] = 1,
    page_size: Annotated[int, Query(ge=1, le=100)] = 20,
    actor_user_id: str | None = None,
    resource_type: str | None = None,
    action: str | None = None,
    start_at: datetime | None = None,
    end_at: datetime | None = None,
) -> Page[AuditLogRead]:
    return AdminService(session, admin).audits(
        page=page,
        page_size=page_size,
        actor_user_id=actor_user_id,
        resource_type=resource_type,
        action=action,
        start_at=start_at,
        end_at=end_at,
    )
