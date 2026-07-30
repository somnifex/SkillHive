from datetime import datetime
from math import ceil

from sqlalchemy import func, or_, select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.db.base import utc_now
from app.models import AuditLog, Group, GroupMember, User
from app.schemas.audit import AuditLogRead
from app.schemas.common import Page
from app.schemas.group import GroupRead, MemberRead
from app.schemas.user import UserRead, UserSummary
from app.services.audit import write_audit


class AdminService:
    def __init__(self, session: Session, admin: User) -> None:
        self.session = session
        self.admin = admin

    def users(
        self,
        *,
        page: int,
        page_size: int,
        query: str | None,
        status: str | None,
    ) -> Page[UserRead]:
        statement = select(User).where(User.status != "deleted")
        if query:
            pattern = f"%{query}%"
            statement = statement.where(
                or_(
                    User.username.ilike(pattern),
                    User.display_name.ilike(pattern),
                    User.email.ilike(pattern),
                )
            )
        if status:
            statement = statement.where(User.status == status)
        total = self.session.scalar(select(func.count()).select_from(statement.subquery())) or 0
        items = self.session.scalars(
            statement.order_by(User.created_at.desc())
            .offset((page - 1) * page_size)
            .limit(page_size)
        )
        return Page[UserRead](
            items=[UserRead.model_validate(user) for user in items],
            page=page,
            page_size=page_size,
            total=total,
            pages=ceil(total / page_size) if total else 0,
        )

    def set_user_status(self, user_id: str, status: str) -> UserRead:
        user = self.session.get(User, user_id)
        if user is None or user.status == "deleted":
            raise AppError("USER_NOT_FOUND", "User was not found.", 404)
        if user.id == self.admin.id and status == "disabled":
            raise AppError("SELF_DISABLE_FORBIDDEN", "You cannot disable your own account.", 409)
        before = user.status
        user.status = status
        write_audit(
            self.session,
            actor_user_id=self.admin.id,
            action=f"user.{status}",
            resource_type="user",
            resource_id=user.id,
            before_data={"status": before},
            after_data={"status": status},
        )
        self.session.commit()
        return UserRead.model_validate(user)

    def groups(
        self,
        *,
        page: int,
        page_size: int,
        query: str | None,
        status: str | None,
    ) -> Page[GroupRead]:
        statement = select(Group)
        if query:
            statement = statement.where(Group.name.ilike(f"%{query}%"))
        if status:
            statement = statement.where(Group.status == status)
        total = self.session.scalar(select(func.count()).select_from(statement.subquery())) or 0
        items = self.session.scalars(
            statement.order_by(Group.created_at.desc())
            .offset((page - 1) * page_size)
            .limit(page_size)
        )
        return Page[GroupRead](
            items=[GroupRead.model_validate(group) for group in items],
            page=page,
            page_size=page_size,
            total=total,
            pages=ceil(total / page_size) if total else 0,
        )

    def group_members(self, group_id: str) -> list[MemberRead]:
        group = self.session.get(Group, group_id)
        if group is None:
            raise AppError("GROUP_NOT_FOUND", "Group was not found.", 404)
        memberships = self.session.scalars(
            select(GroupMember).where(
                GroupMember.group_id == group.id,
                GroupMember.status == "active",
            )
        )
        results: list[MemberRead] = []
        for membership in memberships:
            result = MemberRead.model_validate(membership)
            member = self.session.get(User, membership.user_id)
            result.user = UserSummary.model_validate(member) if member else None
            results.append(result)
        return results

    def set_group_status(self, group_id: str, status: str) -> GroupRead:
        group = self.session.get(Group, group_id)
        if group is None:
            raise AppError("GROUP_NOT_FOUND", "Group was not found.", 404)
        before = group.status
        group.status = status
        group.deleted_at = utc_now() if status == "deleted" else None
        write_audit(
            self.session,
            actor_user_id=self.admin.id,
            action=f"group.{status}_by_admin",
            resource_type="group",
            resource_id=group.id,
            before_data={"status": before},
            after_data={"status": status},
        )
        self.session.commit()
        return GroupRead.model_validate(group)

    def set_group_role(self, group_id: str, user_id: str, role: str) -> MemberRead:
        group = self.session.get(Group, group_id)
        membership = self.session.scalar(
            select(GroupMember).where(
                GroupMember.group_id == group_id,
                GroupMember.user_id == user_id,
                GroupMember.status == "active",
            )
        )
        if group is None or membership is None:
            raise AppError("MEMBER_NOT_FOUND", "Group member was not found.", 404)
        if membership.role == "owner":
            raise AppError("OWNER_ROLE_PROTECTED", "Owner role cannot be changed here.", 409)
        membership.role = role
        write_audit(
            self.session,
            actor_user_id=self.admin.id,
            action="group.role_changed_by_admin",
            resource_type="group",
            resource_id=group.id,
            after_data={"user_id": user_id, "role": role},
        )
        self.session.commit()
        result = MemberRead.model_validate(membership)
        member = self.session.get(User, user_id)
        result.user = UserSummary.model_validate(member) if member else None
        return result

    def audits(
        self,
        *,
        page: int,
        page_size: int,
        actor_user_id: str | None,
        resource_type: str | None,
        action: str | None,
        start_at: datetime | None,
        end_at: datetime | None,
    ) -> Page[AuditLogRead]:
        statement = select(AuditLog)
        if actor_user_id:
            statement = statement.where(AuditLog.actor_user_id == actor_user_id)
        if resource_type:
            statement = statement.where(AuditLog.resource_type == resource_type)
        if action:
            statement = statement.where(AuditLog.action == action)
        if start_at:
            statement = statement.where(AuditLog.created_at >= start_at)
        if end_at:
            statement = statement.where(AuditLog.created_at <= end_at)
        total = self.session.scalar(select(func.count()).select_from(statement.subquery())) or 0
        logs = self.session.scalars(
            statement.order_by(AuditLog.created_at.desc())
            .offset((page - 1) * page_size)
            .limit(page_size)
        )
        return Page[AuditLogRead](
            items=[AuditLogRead.model_validate(log) for log in logs],
            page=page,
            page_size=page_size,
            total=total,
            pages=ceil(total / page_size) if total else 0,
        )
