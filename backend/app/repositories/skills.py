from sqlalchemy import Select, asc, cast, desc, func, or_, select
from sqlalchemy.orm import Session
from sqlalchemy.types import Text

from app.models import Skill, SkillVersion


class SkillRepository:
    def __init__(self, session: Session) -> None:
        self.session = session

    def private_for_owner(self, skill_id: str, owner_id: str) -> Skill | None:
        return self.session.scalar(
            select(Skill).where(
                Skill.id == skill_id,
                Skill.skill_type == "private",
                Skill.owner_user_id == owner_id,
                Skill.status != "deleted",
            )
        )

    def private_for_owner_for_update(self, skill_id: str, owner_id: str) -> Skill | None:
        """Loads an owned Skill with a database row lock where supported."""
        return self.session.scalar(
            select(Skill)
            .where(
                Skill.id == skill_id,
                Skill.skill_type == "private",
                Skill.owner_user_id == owner_id,
                Skill.status != "deleted",
            )
            .with_for_update()
        )

    def global_for_update(self, skill_id: str) -> Skill | None:
        """Loads a global Skill with a database row lock where supported."""
        return self.session.scalar(
            select(Skill)
            .where(
                Skill.id == skill_id,
                Skill.skill_type == "global",
                Skill.status != "deleted",
            )
            .with_for_update()
        )

    def private_trashed_for_owner(self, skill_id: str, owner_id: str) -> Skill | None:
        """Resolves only a trashed skill; active skills never purge/restore."""
        return self.session.scalar(
            select(Skill).where(
                Skill.id == skill_id,
                Skill.skill_type == "private",
                Skill.owner_user_id == owner_id,
                Skill.status == "deleted",
            )
        )

    def private_trashed_for_owner_for_update(
        self, skill_id: str, owner_id: str
    ) -> Skill | None:
        return self.session.scalar(
            select(Skill)
            .where(
                Skill.id == skill_id,
                Skill.skill_type == "private",
                Skill.owner_user_id == owner_id,
                Skill.status == "deleted",
            )
            .with_for_update()
        )

    def slug_exists(self, owner_id: str, slug: str) -> bool:
        # Deliberately NOT status-filtered: the (owner, slug) unique constraint
        # spans trashed rows too, so the guard must match the constraint or an
        # insert would crash with a 500 instead of a clean 409.
        return (
            self.session.scalar(
                select(Skill.id).where(
                    Skill.owner_user_id == owner_id,
                    Skill.slug == slug,
                )
            )
            is not None
        )

    def group_slug_exists(self, group_id: str, slug: str) -> bool:
        """Match the group/slug unique constraint, including trashed rows."""
        return (
            self.session.scalar(
                select(Skill.id).where(
                    Skill.group_id == group_id,
                    Skill.slug == slug,
                )
            )
            is not None
        )

    def group_for_member(self, skill_id: str, group_id: str) -> Skill | None:
        return self.session.scalar(
            select(Skill).where(
                Skill.id == skill_id,
                Skill.skill_type == "group",
                Skill.group_id == group_id,
                Skill.status != "deleted",
            )
        )

    def group_for_member_for_update(self, skill_id: str, group_id: str) -> Skill | None:
        return self.session.scalar(
            select(Skill)
            .where(
                Skill.id == skill_id,
                Skill.skill_type == "group",
                Skill.group_id == group_id,
                Skill.status != "deleted",
            )
            .with_for_update()
        )

    def group_trashed_for_update(self, skill_id: str, group_id: str) -> Skill | None:
        return self.session.scalar(
            select(Skill)
            .where(
                Skill.id == skill_id,
                Skill.skill_type == "group",
                Skill.group_id == group_id,
                Skill.status == "deleted",
            )
            .with_for_update()
        )

    def list_group(
        self,
        group_id: str,
        *,
        page: int,
        page_size: int,
        query: str | None,
        category: str | None,
        tag: str | None,
        status: str | None,
        sort: str,
        order: str,
    ) -> tuple[list[Skill], int]:
        statement: Select[tuple[Skill]] = select(Skill).where(
            Skill.skill_type == "group",
            Skill.group_id == group_id,
            Skill.status != "deleted",
        )
        if query:
            pattern = f"%{query.strip()}%"
            statement = statement.where(
                or_(Skill.name.ilike(pattern), Skill.description.ilike(pattern))
            )
        if category:
            statement = statement.where(Skill.category == category)
        if tag:
            statement = statement.where(cast(Skill.tags, Text).contains(f'"{tag}"'))
        if status:
            statement = statement.where(Skill.status == status)
        total = self.session.scalar(select(func.count()).select_from(statement.subquery())) or 0
        sort_column = {
            "name": Skill.name,
            "created_at": Skill.created_at,
            "updated_at": Skill.updated_at,
        }.get(sort, Skill.updated_at)
        ordering = desc(sort_column) if order == "desc" else asc(sort_column)
        items = list(
            self.session.scalars(
                statement.order_by(ordering).offset((page - 1) * page_size).limit(page_size)
            )
        )
        return items, total

    def list_group_trash(
        self,
        group_id: str,
        *,
        page: int,
        page_size: int,
        query: str | None,
    ) -> tuple[list[Skill], int]:
        statement: Select[tuple[Skill]] = select(Skill).where(
            Skill.skill_type == "group",
            Skill.group_id == group_id,
            Skill.status == "deleted",
        )
        if query:
            pattern = f"%{query.strip()}%"
            statement = statement.where(
                or_(Skill.name.ilike(pattern), Skill.description.ilike(pattern))
            )
        total = self.session.scalar(select(func.count()).select_from(statement.subquery())) or 0
        items = list(
            self.session.scalars(
                statement
                .order_by(desc(Skill.deleted_at))
                .offset((page - 1) * page_size)
                .limit(page_size)
            )
        )
        return items, total

    def list_private(
        self,
        owner_id: str,
        *,
        page: int,
        page_size: int,
        query: str | None,
        category: str | None,
        tag: str | None,
        status: str | None,
        sort: str,
        order: str,
    ) -> tuple[list[Skill], int]:
        statement: Select[tuple[Skill]] = select(Skill).where(
            Skill.skill_type == "private",
            Skill.owner_user_id == owner_id,
            Skill.status != "deleted",
        )
        if query:
            pattern = f"%{query.strip()}%"
            statement = statement.where(
                or_(Skill.name.ilike(pattern), Skill.description.ilike(pattern))
            )
        if category:
            statement = statement.where(Skill.category == category)
        if tag:
            statement = statement.where(cast(Skill.tags, Text).contains(f'"{tag}"'))
        if status:
            statement = statement.where(Skill.status == status)
        total = self.session.scalar(select(func.count()).select_from(statement.subquery())) or 0
        sort_column = {
            "name": Skill.name,
            "created_at": Skill.created_at,
            "updated_at": Skill.updated_at,
        }.get(sort, Skill.updated_at)
        ordering = desc(sort_column) if order == "desc" else asc(sort_column)
        items = list(
            self.session.scalars(
                statement.order_by(ordering).offset((page - 1) * page_size).limit(page_size)
            )
        )
        return items, total

    def list_private_trash(
        self,
        owner_id: str,
        *,
        page: int,
        page_size: int,
        query: str | None,
    ) -> tuple[list[Skill], int]:
        statement: Select[tuple[Skill]] = select(Skill).where(
            Skill.skill_type == "private",
            Skill.owner_user_id == owner_id,
            Skill.status == "deleted",
        )
        if query:
            pattern = f"%{query.strip()}%"
            statement = statement.where(
                or_(Skill.name.ilike(pattern), Skill.description.ilike(pattern))
            )
        total = self.session.scalar(select(func.count()).select_from(statement.subquery())) or 0
        items = list(
            self.session.scalars(
                statement
                .order_by(desc(Skill.deleted_at))
                .offset((page - 1) * page_size)
                .limit(page_size)
            )
        )
        return items, total

    def private_categories(self, owner_id: str) -> list[str]:
        """Distinct non-empty categories across the owner's active skills."""
        rows = self.session.execute(
            select(Skill.category)
            .where(
                Skill.skill_type == "private",
                Skill.owner_user_id == owner_id,
                Skill.status != "deleted",
                Skill.category != "",
            )
            .distinct()
            .order_by(asc(Skill.category))
        ).scalars()
        return list(rows)

    def version_by_name(self, skill_id: str, version: str) -> SkillVersion | None:
        return self.session.scalar(
            select(SkillVersion).where(
                SkillVersion.skill_id == skill_id,
                SkillVersion.version == version,
            )
        )

    def version_by_name_for_update(self, skill_id: str, version: str) -> SkillVersion | None:
        return self.session.scalar(
            select(SkillVersion)
            .where(
                SkillVersion.skill_id == skill_id,
                SkillVersion.version == version,
            )
            .with_for_update()
        )

    def version_for_update(self, version_id: str | None, skill_id: str) -> SkillVersion | None:
        if version_id is None:
            return None
        return self.session.scalar(
            select(SkillVersion)
            .where(SkillVersion.id == version_id, SkillVersion.skill_id == skill_id)
            .with_for_update()
        )

    def version(self, version_id: str | None) -> SkillVersion | None:
        return self.session.get(SkillVersion, version_id) if version_id else None

    def versions(self, skill_id: str) -> list[SkillVersion]:
        return list(
            self.session.scalars(
                select(SkillVersion)
                .where(SkillVersion.skill_id == skill_id)
                .order_by(SkillVersion.created_at.desc())
            )
        )
