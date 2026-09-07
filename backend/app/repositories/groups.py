from collections.abc import Iterable, Sequence

from sqlalchemy import func, literal, select
from sqlalchemy.orm import Session, aliased

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

    def list_all_active(
        self,
        *,
        page: int,
        page_size: int,
    ) -> tuple[list[Group], int]:
        statement = select(Group).where(Group.status == "active")
        total = self.session.scalar(select(func.count()).select_from(statement.subquery())) or 0
        rows = list(
            self.session.scalars(
                statement.order_by(Group.created_at.desc())
                .offset((page - 1) * page_size)
                .limit(page_size)
            )
        )
        return rows, total

    def list_active_all(self) -> list[Group]:
        return list(
            self.session.scalars(
                select(Group).where(Group.status == "active").order_by(Group.created_at)
            )
        )

    def get_active_by_ids(self, group_ids: Sequence[str]) -> list[Group]:
        if not group_ids:
            return []
        return list(
            self.session.scalars(
                select(Group)
                .where(Group.id.in_(group_ids), Group.status == "active")
                .order_by(Group.created_at)
            )
        )

    def names_for(self, group_ids: Iterable[str]) -> dict[str, str]:
        ids = [gid for gid in group_ids if gid]
        if not ids:
            return {}
        rows = self.session.execute(select(Group.id, Group.name).where(Group.id.in_(ids))).all()
        return {row[0]: row[1] for row in rows}

    def effective_roles(self, group_ids: Sequence[str], user_id: str) -> dict[str, str]:
        """Effective role per requested group, walking each ancestor chain.

        A group resolves to its own membership role unless an ancestor grants
        manager rights, which lift a plain (or absent) membership to ``admin``.
        Inheritance never produces ``owner``. Inactive ancestors break the
        chain. Groups where the user has no relationship at all are omitted.
        """
        if not group_ids:
            return {}
        chain = (
            select(
                Group.id.label("root_id"),
                Group.id.label("group_id"),
                Group.parent_id.label("parent_id"),
            )
            .where(Group.id.in_(list(group_ids)), Group.status == "active")
            .cte(name="group_role_chain", recursive=True)
        )
        ancestor = aliased(Group)
        chain = chain.union_all(
            select(
                chain.c.root_id,
                ancestor.id,
                ancestor.parent_id,
            )
            .join(chain, chain.c.parent_id == ancestor.id)
            .where(ancestor.status == "active")
        )
        rows = self.session.execute(
            select(chain.c.root_id, chain.c.group_id, GroupMember.role).join(
                GroupMember,
                (GroupMember.group_id == chain.c.group_id)
                & (GroupMember.user_id == user_id)
                & (GroupMember.status == "active"),
            )
        ).all()
        own: dict[str, str] = {}
        inherited_manager: dict[str, bool] = {gid: False for gid in group_ids}
        for root_id, group_id, role in rows:
            if group_id == root_id:
                own[root_id] = role
            elif role in ("owner", "admin"):
                inherited_manager[root_id] = True
        roles: dict[str, str] = {}
        for group_id in group_ids:
            role = own.get(group_id)
            if role == "owner":
                roles[group_id] = "owner"
            elif role == "admin" or inherited_manager.get(group_id):
                roles[group_id] = "admin"
            elif role == "member":
                roles[group_id] = "member"
        return roles

    def ancestor_ids(self, group_id: str) -> list[str]:
        """Chain from the group itself up to the root, inclusive."""
        chain = (
            select(Group.id.label("group_id"), Group.parent_id.label("parent_id"))
            .where(Group.id == group_id)
            .cte(name="group_ancestor_chain", recursive=True)
        )
        ancestor = aliased(Group)
        chain = chain.union_all(
            select(ancestor.id, ancestor.parent_id).join(chain, ancestor.id == chain.c.parent_id)
        )
        return [row[0] for row in self.session.execute(select(chain.c.group_id))]

    def descendant_ids(self, group_id: str) -> set[str]:
        """The group itself plus every descendant, regardless of status."""
        chain = (
            select(Group.id.label("group_id"))
            .where(Group.id == group_id)
            .cte(name="group_subtree", recursive=True)
        )
        child = aliased(Group)
        chain = chain.union_all(
            select(child.id).join(chain, child.parent_id == chain.c.group_id)
        )
        return {row[0] for row in self.session.execute(select(chain.c.group_id))}

    def subtree_depth(self, group_id: str) -> int:
        """Levels from the group down to its deepest descendant (leaf = 1)."""
        chain = (
            select(Group.id.label("group_id"), literal(1).label("depth"))
            .where(Group.id == group_id)
            .cte(name="group_depth", recursive=True)
        )
        child = aliased(Group)
        chain = chain.union_all(
            select(child.id, (chain.c.depth + 1).label("depth")).join(
                chain, child.parent_id == chain.c.group_id
            )
        )
        return self.session.scalar(select(func.max(chain.c.depth))) or 0

    def active_child_count(self, group_id: str) -> int:
        return (
            self.session.scalar(
                select(func.count())
                .select_from(Group)
                .where(Group.parent_id == group_id, Group.status == "active")
            )
            or 0
        )

    def visible_group_ids(self, user_id: str) -> list[str]:
        """Groups the user may see in the tree: direct memberships plus the
        full active subtree of every group the user manages directly."""
        membership_rows = self.session.execute(
            select(Group.id, GroupMember.role)
            .join(Group, Group.id == GroupMember.group_id)
            .where(
                GroupMember.user_id == user_id,
                GroupMember.status == "active",
                Group.status == "active",
            )
        ).all()
        direct_ids: list[str] = []
        manager_ids: list[str] = []
        for group_id, role in membership_rows:
            direct_ids.append(group_id)
            if role in ("owner", "admin"):
                manager_ids.append(group_id)
        if not manager_ids:
            return direct_ids
        subtree = (
            select(Group.id.label("group_id"))
            .where(Group.id.in_(manager_ids), Group.status == "active")
            .cte(name="group_visible_subtree", recursive=True)
        )
        child = aliased(Group)
        subtree = subtree.union_all(
            select(child.id)
            .join(subtree, child.parent_id == subtree.c.group_id)
            .where(child.status == "active")
        )
        subtree_ids = {row[0] for row in self.session.execute(select(subtree.c.group_id))}
        return list({*direct_ids, *subtree_ids})
