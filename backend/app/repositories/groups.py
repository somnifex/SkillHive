from sqlalchemy import func, select
from sqlalchemy.orm import Session

from app.models import Group, GroupMember


class GroupRepository:
    def __init__(self, session: Session) -> None:
        self.session = session

    def get_active(self, group_id: str) -> Group | None:
        return self.session.scalar(
            select(Group).where(Group.id == group_id, Group.status == "active")
        )

    def membership(self, group_id: str, user_id: str) -> GroupMember | None:
        return self.session.scalar(
            select(GroupMember).where(
                GroupMember.group_id == group_id,
                GroupMember.user_id == user_id,
                GroupMember.status == "active",
            )
        )

    def list_for_user(
        self,
        user_id: str,
        *,
        page: int,
        page_size: int,
        managed_only: bool,
    ) -> tuple[list[tuple[Group, str]], int]:
        statement = (
            select(Group, GroupMember.role)
            .join(GroupMember, GroupMember.group_id == Group.id)
            .where(
                GroupMember.user_id == user_id,
                GroupMember.status == "active",
                Group.status == "active",
            )
        )
        if managed_only:
            statement = statement.where(GroupMember.role.in_(["owner", "admin"]))
        total = self.session.scalar(select(func.count()).select_from(statement.subquery())) or 0
        rows = list(
            self.session.execute(
                statement.order_by(Group.updated_at.desc())
                .offset((page - 1) * page_size)
                .limit(page_size)
            ).tuples()
        )
        return rows, total
