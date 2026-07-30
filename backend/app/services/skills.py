from math import ceil

from sqlalchemy import select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.models import Skill, SkillVersion, User
from app.repositories.skills import SkillRepository
from app.schemas.common import Page
from app.schemas.skill import (
    SkillCreate,
    SkillRead,
    SkillUpdate,
    SkillVersionCreate,
    SkillVersionRead,
)
from app.services.audit import write_audit


class PrivateSkillService:
    def __init__(self, session: Session, user: User) -> None:
        self.session = session
        self.user = user
        self.repository = SkillRepository(session)

    def list_page(
        self,
        *,
        page: int,
        page_size: int,
        query: str | None,
        category: str | None,
        tag: str | None,
        status: str | None,
        sort: str,
        order: str,
    ) -> Page[SkillRead]:
        items, total = self.repository.list_private(
            self.user.id,
            page=page,
            page_size=page_size,
            query=query,
            category=category,
            tag=tag,
            status=status,
            sort=sort,
            order=order,
        )
        return Page[SkillRead](
            items=[self._read(skill, include_content=False) for skill in items],
            page=page,
            page_size=page_size,
            total=total,
            pages=ceil(total / page_size) if total else 0,
        )

    def create(self, data: SkillCreate) -> SkillRead:
        if self.repository.slug_exists(self.user.id, data.slug):
            raise AppError("SKILL_SLUG_TAKEN", "A skill with this slug already exists.", 409)
        skill = Skill(
            name=data.name.strip(),
            slug=data.slug,
            description=data.description,
            skill_type="private",
            owner_user_id=self.user.id,
            category=data.category,
            tags=data.tags,
            status="draft",
            created_by=self.user.id,
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
            created_by=self.user.id,
        )
        self.session.add(version)
        self.session.flush()
        skill.current_version_id = version.id
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="private_skill.created",
            resource_type="skill",
            resource_id=skill.id,
            after_data={"name": skill.name, "version": version.version},
        )
        self.session.commit()
        return self._read(skill)

    def get(self, skill_id: str) -> SkillRead:
        return self._read(self._owned(skill_id))

    def update(self, skill_id: str, data: SkillUpdate) -> SkillRead:
        skill = self._owned(skill_id)
        before = {"name": skill.name, "status": skill.status, "version": skill.current_version_id}
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
                created_by=self.user.id,
            )
            self.session.add(version)
            self.session.flush()
            skill.current_version_id = version.id
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="private_skill.updated",
            resource_type="skill",
            resource_id=skill.id,
            before_data=before,
            after_data={"name": skill.name, "status": skill.status},
        )
        self.session.commit()
        return self._read(skill)

    def create_version(self, skill_id: str, data: SkillVersionCreate) -> SkillVersionRead:
        skill = self._owned(skill_id)
        self._ensure_version_available(skill.id, data.version)
        version = SkillVersion(
            skill_id=skill.id,
            version=data.version,
            content=data.content.model_dump(mode="json"),
            manifest=data.manifest,
            dependency_config=data.dependency_config,
            change_log=data.change_log,
            status=data.status,
            created_by=self.user.id,
        )
        self.session.add(version)
        self.session.flush()
        skill.current_version_id = version.id
        if data.status == "published":
            skill.status = "published"
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="private_skill.version_created",
            resource_type="skill",
            resource_id=skill.id,
            after_data={"version": version.version, "status": version.status},
        )
        self.session.commit()
        return SkillVersionRead.model_validate(version)

    def versions(self, skill_id: str) -> list[SkillVersionRead]:
        skill = self._owned(skill_id)
        return [
            SkillVersionRead.model_validate(version)
            for version in self.repository.versions(skill.id)
        ]

    def copy(self, skill_id: str) -> SkillRead:
        source = self._owned(skill_id)
        source_version = self.repository.version(source.current_version_id)
        base_slug = f"{source.slug}-copy"
        slug = base_slug
        suffix = 2
        while self.repository.slug_exists(self.user.id, slug):
            slug = f"{base_slug}-{suffix}"
            suffix += 1
        content = source_version.content if source_version else {}
        return self.create(
            SkillCreate(
                name=f"{source.name} 副本",
                slug=slug,
                description=source.description,
                category=source.category,
                tags=source.tags,
                content=content,
                version=source_version.version if source_version else "0.1.0",
                change_log="Copied from an existing skill",
            )
        )

    def delete(self, skill_id: str) -> None:
        skill = self._owned(skill_id)
        skill.status = "deleted"
        from app.db.base import utc_now

        skill.deleted_at = utc_now()
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="private_skill.deleted",
            resource_type="skill",
            resource_id=skill.id,
            before_data={"name": skill.name},
        )
        self.session.commit()

    def _owned(self, skill_id: str) -> Skill:
        skill = self.repository.private_for_owner(skill_id, self.user.id)
        if skill is None:
            raise AppError("SKILL_NOT_FOUND", "Skill was not found.", 404)
        return skill

    def _read(self, skill: Skill, *, include_content: bool = True) -> SkillRead:
        current = self.repository.version(skill.current_version_id) if include_content else None
        result = SkillRead.model_validate(skill)
        result.current_version = SkillVersionRead.model_validate(current) if current else None
        return result

    def _ensure_version_available(self, skill_id: str, version: str) -> None:
        exists = self.session.scalar(
            select(SkillVersion.id).where(
                SkillVersion.skill_id == skill_id,
                SkillVersion.version == version,
            )
        )
        if exists is not None:
            raise AppError("VERSION_EXISTS", "This version already exists.", 409)

    def _next_version(self, skill: Skill) -> str:
        current = self.repository.version(skill.current_version_id)
        if current is None:
            return "0.1.0"
        base = current.version.split("-", 1)[0]
        try:
            major, minor, patch = (int(part) for part in base.split("."))
        except ValueError:
            return "0.1.0"
        return f"{major}.{minor}.{patch + 1}"
