from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from sqlalchemy import select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.db.base import utc_now
from app.models import Skill, SkillVersion
from app.services.audit import write_audit


class SkillMutationService:
    """Shared transactional mutation path for Skill resources.

    This service deliberately never commits or rolls back the SQLAlchemy session.
    The caller owns the transaction boundary. Existing REST facades commit after a
    successful domain mutation; the M2 sync path will append revision/change-feed/
    receipt rows to the same transaction before committing.

    Authorization is also deliberately outside this class. A caller must resolve
    an already-authorized Skill before passing it to update/delete/version methods.
    """

    def __init__(self, session: Session, actor_user_id: str) -> None:
        self.session = session
        self.actor_user_id = actor_user_id

    def create_skill(
        self,
        *,
        name: str,
        slug: str,
        description: str,
        skill_type: str,
        owner_user_id: str | None,
        category: str,
        tags: list[str],
        skill_status: str,
        version: str,
        content: Mapping[str, Any],
        manifest: Mapping[str, Any],
        dependency_config: Mapping[str, Any],
        change_log: str,
        version_status: str,
        audit_action: str,
        audit_after_data: Mapping[str, Any] | None = None,
    ) -> tuple[Skill, SkillVersion]:
        skill = Skill(
            name=name.strip(),
            slug=slug,
            description=description,
            skill_type=skill_type,
            owner_user_id=owner_user_id,
            category=category,
            tags=list(tags),
            status=skill_status,
            created_by=self.actor_user_id,
        )
        self.session.add(skill)
        self.session.flush()

        created_version = self._create_version_row(
            skill=skill,
            version=version,
            content=content,
            manifest=manifest,
            dependency_config=dependency_config,
            change_log=change_log,
            status=version_status,
        )
        skill.current_version_id = created_version.id

        after_data: dict[str, Any] = {
            "name": skill.name,
            "version": created_version.version,
        }
        if audit_after_data is not None:
            after_data.update(audit_after_data)
        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            after_data=after_data,
        )
        return skill, created_version

    def update_skill(
        self,
        skill: Skill,
        *,
        updates: Mapping[str, Any],
        audit_action: str,
        content: Mapping[str, Any] | None = None,
        version: str | None = None,
        manifest: Mapping[str, Any] | None = None,
        dependency_config: Mapping[str, Any] | None = None,
        change_log: str = "",
        version_status: str = "draft",
        demote_published_on_new_version: bool = False,
    ) -> SkillVersion | None:
        before = {
            "name": skill.name,
            "status": skill.status,
            "version": skill.current_version_id,
        }

        for key, value in updates.items():
            if value is not None:
                setattr(skill, key, value)

        created_version: SkillVersion | None = None
        if content is not None:
            version_name = version or self.next_patch_version(skill)
            self.ensure_version_available(skill.id, version_name)
            version_manifest = (
                manifest if manifest is not None else {"name": skill.slug, "schema_version": 1}
            )
            created_version = self._create_version_row(
                skill=skill,
                version=version_name,
                content=content,
                manifest=version_manifest,
                dependency_config=dependency_config or {},
                change_log=change_log,
                status=version_status,
            )
            skill.current_version_id = created_version.id
            if demote_published_on_new_version and skill.status == "published":
                skill.status = "draft"

        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            before_data=before,
            after_data={"name": skill.name, "status": skill.status},
        )
        return created_version

    def create_version(
        self,
        skill: Skill,
        *,
        version: str,
        content: Mapping[str, Any],
        manifest: Mapping[str, Any],
        dependency_config: Mapping[str, Any],
        change_log: str,
        version_status: str,
        audit_action: str,
        publish_skill: bool = False,
        demote_published_skill: bool = False,
    ) -> SkillVersion:
        self.ensure_version_available(skill.id, version)
        created_version = self._create_version_row(
            skill=skill,
            version=version,
            content=content,
            manifest=manifest,
            dependency_config=dependency_config,
            change_log=change_log,
            status=version_status,
        )
        skill.current_version_id = created_version.id

        if publish_skill:
            skill.status = "published"
        elif demote_published_skill and skill.status == "published":
            skill.status = "draft"

        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            after_data={"version": created_version.version, "status": created_version.status},
        )
        return created_version

    def publish_version(
        self,
        skill: Skill,
        version: SkillVersion,
        *,
        audit_action: str,
    ) -> None:
        if version.skill_id != skill.id:
            raise AppError("VERSION_NOT_FOUND", "Skill version was not found.", 404)
        version.status = "published"
        skill.current_version_id = version.id
        skill.status = "published"
        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            after_data={"version": version.version},
        )

    def set_status(self, skill: Skill, status: str, *, audit_action: str) -> None:
        before_status = skill.status
        skill.status = status
        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            before_data={"status": before_status},
            after_data={"status": status},
        )

    def soft_delete(self, skill: Skill, *, audit_action: str) -> None:
        before = {"name": skill.name, "status": skill.status}
        skill.status = "deleted"
        skill.deleted_at = utc_now()
        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            before_data=before,
            after_data={"status": "deleted"},
        )

    def ensure_version_available(self, skill_id: str, version: str) -> None:
        exists = self.session.scalar(
            select(SkillVersion.id).where(
                SkillVersion.skill_id == skill_id,
                SkillVersion.version == version,
            )
        )
        if exists is not None:
            raise AppError("VERSION_EXISTS", "This version already exists.", 409)

    def next_patch_version(self, skill: Skill) -> str:
        if skill.current_version_id is None:
            return "0.1.0"
        current = self.session.get(SkillVersion, skill.current_version_id)
        if current is None:
            return "0.1.0"
        base = current.version.split("-", 1)[0]
        try:
            major, minor, patch = (int(part) for part in base.split("."))
        except ValueError:
            return "0.1.0"
        return f"{major}.{minor}.{patch + 1}"

    def _create_version_row(
        self,
        *,
        skill: Skill,
        version: str,
        content: Mapping[str, Any],
        manifest: Mapping[str, Any],
        dependency_config: Mapping[str, Any],
        change_log: str,
        status: str,
    ) -> SkillVersion:
        created_version = SkillVersion(
            skill_id=skill.id,
            version=version,
            content=dict(content),
            manifest=dict(manifest),
            dependency_config=dict(dependency_config),
            change_log=change_log,
            status=status,
            created_by=self.actor_user_id,
        )
        self.session.add(created_version)
        self.session.flush()
        return created_version
