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

    def slug_exists(self, owner_id: str, slug: str) -> bool:
        return (
            self.session.scalar(
                select(Skill.id).where(
                    Skill.owner_user_id == owner_id,
                    Skill.slug == slug,
                    Skill.status != "deleted",
                )
            )
            is not None
        )

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
