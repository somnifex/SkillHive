from math import ceil

from sqlalchemy import func, or_, select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.db.base import utc_now
from app.models import GroupSkillGrant, Skill, SkillVersion, User
from app.repositories.skills import SkillRepository
from app.schemas.common import Page
from app.schemas.skill import (
    GlobalSkillCreate,
    GlobalSkillUpdate,
    GroupSkillGrantRead,
    SkillRead,
    SkillVersionCreate,
    SkillVersionRead,
)
from app.services.audit import write_audit


class GlobalSkillService:
    def __init__(self, session: Session, admin: User) -> None:
        self.session = session
        self.admin = admin
        self.repository = SkillRepository(session)

    def list_page(
        self,
        *,
        page: int,
        page_size: int,
        query: str | None,
        status: str | None,
    ) -> Page[SkillRead]:
        statement = select(Skill).where(Skill.skill_type == "global", Skill.status != "deleted")
        if query:
            pattern = f"%{query}%"
            statement = statement.where(
                or_(Skill.name.ilike(pattern), Skill.description.ilike(pattern))
            )
        if status:
            statement = statement.where(Skill.status == status)
        total = self.session.scalar(select(func.count()).select_from(statement.subquery())) or 0
        items = self.session.scalars(
            statement.order_by(Skill.updated_at.desc())
            .offset((page - 1) * page_size)
            .limit(page_size)
        )
        return Page[SkillRead](
            items=[self._read(skill, include_version=False) for skill in items],
            page=page,
            page_size=page_size,
            total=total,
            pages=ceil(total / page_size) if total else 0,
        )

    def create(self, data: GlobalSkillCreate) -> SkillRead:
        duplicate = self.session.scalar(
            select(Skill.id).where(
                Skill.skill_type == "global",
                Skill.slug == data.slug,
                Skill.status != "deleted",
            )
        )
        if duplicate:
            raise AppError("SKILL_SLUG_TAKEN", "A global skill with this slug exists.", 409)
        skill = Skill(
            name=data.name.strip(),
            slug=data.slug,
            description=data.description,
            skill_type="global",
            owner_user_id=None,
            category=data.category,
            tags=data.tags,
            status="draft",
            created_by=self.admin.id,
        )
        self.session.add(skill)
        self.session.flush()
        version = SkillVersion(
            skill_id=skill.id,
            version=data.version,
            content=data.content.model_dump(mode="json"),
            manifest={"name": skill.slug, "schema_version": 1},
            dependency_config={},
            change_log=data.change_log,
            status="draft",
            created_by=self.admin.id,
        )
        self.session.add(version)
        self.session.flush()
        skill.current_version_id = version.id
        write_audit(
            self.session,
            actor_user_id=self.admin.id,
            action="global_skill.created",
            resource_type="skill",
            resource_id=skill.id,
            after_data={"name": skill.name, "version": version.version},
        )
        self.session.commit()
        return self._read(skill)

    def get(self, skill_id: str) -> SkillRead:
        return self._read(self._global(skill_id))

    def update(self, skill_id: str, data: GlobalSkillUpdate) -> SkillRead:
        skill = self._global(skill_id)
        updates = data.model_dump(exclude_unset=True, exclude={"content", "version", "change_log"})
        for key, value in updates.items():
            if value is not None:
                setattr(skill, key, value)
        if data.content is not None:
            version_name = data.version or self._next_version(skill)
            self._ensure_version_available(skill.id, version_name)
            version = SkillVersion(
                skill_id=skill.id,
                version=version_name,
                content=data.content.model_dump(mode="json"),
                manifest={"name": skill.slug, "schema_version": 1},
                dependency_config={},
                change_log=data.change_log,
                status="draft",
                created_by=self.admin.id,
            )
            self.session.add(version)
            self.session.flush()
            skill.current_version_id = version.id
            if skill.status == "published":
                skill.status = "draft"
        write_audit(
            self.session,
            actor_user_id=self.admin.id,
            action="global_skill.updated",
            resource_type="skill",
            resource_id=skill.id,
            after_data={"name": skill.name, "status": skill.status},
        )
        self.session.commit()
        return self._read(skill)

    def create_version(self, skill_id: str, data: SkillVersionCreate) -> SkillVersionRead:
        skill = self._global(skill_id)
        self._ensure_version_available(skill.id, data.version)
        version = SkillVersion(
            skill_id=skill.id,
            version=data.version,
            content=data.content.model_dump(mode="json"),
            manifest=data.manifest,
            dependency_config=data.dependency_config,
            change_log=data.change_log,
            status="draft",
            created_by=self.admin.id,
        )
        self.session.add(version)
        self.session.flush()
        skill.current_version_id = version.id
        if skill.status == "published":
            skill.status = "draft"
        self.session.commit()
        return SkillVersionRead.model_validate(version)

    def publish(self, skill_id: str, version_id: str | None) -> SkillRead:
        skill = self._global(skill_id)
        selected_id = version_id or skill.current_version_id
        version = self.session.scalar(
            select(SkillVersion).where(
                SkillVersion.id == selected_id,
                SkillVersion.skill_id == skill.id,
            )
        )
        if version is None:
            raise AppError("VERSION_NOT_FOUND", "Skill version was not found.", 404)
        version.status = "published"
        skill.current_version_id = version.id
        skill.status = "published"
        write_audit(
            self.session,
            actor_user_id=self.admin.id,
            action="global_skill.published",
            resource_type="skill",
            resource_id=skill.id,
            after_data={"version": version.version},
        )
        self.session.commit()
        return self._read(skill)

    def set_status(self, skill_id: str, status: str) -> SkillRead:
        skill = self._global(skill_id)
        before = skill.status
        skill.status = status
        write_audit(
            self.session,
            actor_user_id=self.admin.id,
            action=f"global_skill.{status}",
            resource_type="skill",
            resource_id=skill.id,
            before_data={"status": before},
            after_data={"status": status},
        )
        self.session.commit()
        return self._read(skill)

    def versions(self, skill_id: str) -> list[SkillVersionRead]:
        skill = self._global(skill_id)
        return [
            SkillVersionRead.model_validate(item) for item in self.repository.versions(skill.id)
        ]

    def grants(self, skill_id: str) -> list[GroupSkillGrantRead]:
        skill = self._global(skill_id)
        grants = self.session.scalars(
            select(GroupSkillGrant).where(GroupSkillGrant.skill_id == skill.id)
        )
        return [self._grant_read(grant) for grant in grants]

    def revoke_grant(self, skill_id: str, group_id: str) -> None:
        skill = self._global(skill_id)
        grant = self.session.scalar(
            select(GroupSkillGrant).where(
                GroupSkillGrant.skill_id == skill.id,
                GroupSkillGrant.group_id == group_id,
                GroupSkillGrant.status != "revoked",
            )
        )
        if grant is None:
            raise AppError("GRANT_NOT_FOUND", "Grant was not found.", 404)
        grant.status = "revoked"
        grant.revoked_by = self.admin.id
        grant.revoked_at = utc_now()
        write_audit(
            self.session,
            actor_user_id=self.admin.id,
            action="group_skill.revoked_by_admin",
            resource_type="group_skill_grant",
            resource_id=grant.id,
        )
        self.session.commit()

    def _global(self, skill_id: str) -> Skill:
        skill = self.session.scalar(
            select(Skill).where(
                Skill.id == skill_id,
                Skill.skill_type == "global",
                Skill.status != "deleted",
            )
        )
        if skill is None:
            raise AppError("SKILL_NOT_FOUND", "Global skill was not found.", 404)
        return skill

    def _read(self, skill: Skill, *, include_version: bool = True) -> SkillRead:
        result = SkillRead.model_validate(skill)
        version = self.repository.version(skill.current_version_id) if include_version else None
        result.current_version = SkillVersionRead.model_validate(version) if version else None
        return result

    def _grant_read(self, grant: GroupSkillGrant) -> GroupSkillGrantRead:
        result = GroupSkillGrantRead.model_validate(grant)
        result.skill = self._read(self._global(grant.skill_id), include_version=False)
        version_id = (
            grant.locked_version_id
            if grant.version_policy == "locked"
            else result.skill.current_version_id
        )
        version = self.repository.version(version_id)
        result.effective_version = SkillVersionRead.model_validate(version) if version else None
        return result

    def _ensure_version_available(self, skill_id: str, version: str) -> None:
        if self.session.scalar(
            select(SkillVersion.id).where(
                SkillVersion.skill_id == skill_id,
                SkillVersion.version == version,
            )
        ):
            raise AppError("VERSION_EXISTS", "This version already exists.", 409)

    def _next_version(self, skill: Skill) -> str:
        current = self.repository.version(skill.current_version_id)
        if current is None:
            return "0.1.0"
        major, minor, patch = (int(part) for part in current.version.split("-", 1)[0].split("."))
        return f"{major}.{minor}.{patch + 1}"
