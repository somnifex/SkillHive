from sqlalchemy import select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.db.base import utc_now
from app.models import GroupMember, GroupSkillGrant, Skill, SkillVersion, User
from app.repositories.groups import GroupRepository
from app.repositories.skills import SkillRepository
from app.schemas.skill import (
    GroupSkillGrantCreate,
    GroupSkillGrantRead,
    GroupSkillGrantUpdate,
    SkillRead,
    SkillVersionRead,
)
from app.services.audit import write_audit


class GroupSkillService:
    def __init__(self, session: Session, user: User) -> None:
        self.session = session
        self.user = user
        self.groups = GroupRepository(session)
        self.skills = SkillRepository(session)

    def enabled(self, group_id: str) -> list[GroupSkillGrantRead]:
        self._membership(group_id)
        grants = self.session.scalars(
            select(GroupSkillGrant)
            .join(Skill, Skill.id == GroupSkillGrant.skill_id)
            .where(
                GroupSkillGrant.group_id == group_id,
                GroupSkillGrant.status == "active",
                Skill.skill_type == "global",
                Skill.status == "published",
            )
            .order_by(Skill.name)
        )
        return [self._read(grant) for grant in grants]

    def catalog(self, group_id: str) -> list[SkillRead]:
        self._manager(group_id)
        skills = self.session.scalars(
            select(Skill)
            .where(Skill.skill_type == "global", Skill.status == "published")
            .order_by(Skill.name)
        )
        return [self._skill_read(skill) for skill in skills]

    def catalog_versions(self, group_id: str, skill_id: str) -> list[SkillVersionRead]:
        self._manager(group_id)
        skill = self._published_global(skill_id)
        versions = self.session.scalars(
            select(SkillVersion)
            .where(
                SkillVersion.skill_id == skill.id,
                SkillVersion.status == "published",
            )
            .order_by(SkillVersion.created_at.desc())
        )
        return [SkillVersionRead.model_validate(version) for version in versions]

    def grant(
        self,
        group_id: str,
        skill_id: str,
        data: GroupSkillGrantCreate,
    ) -> GroupSkillGrantRead:
        self._manager(group_id)
        skill = self._published_global(skill_id)
        self._validate_policy(skill, data.version_policy, data.locked_version_id)
        grant = self.session.scalar(
            select(GroupSkillGrant).where(
                GroupSkillGrant.group_id == group_id,
                GroupSkillGrant.skill_id == skill.id,
            )
        )
        if grant is None:
            grant = GroupSkillGrant(
                group_id=group_id,
                skill_id=skill.id,
                version_policy=data.version_policy,
                locked_version_id=data.locked_version_id,
                status="active",
                granted_by=self.user.id,
            )
            self.session.add(grant)
        else:
            grant.version_policy = data.version_policy
            grant.locked_version_id = data.locked_version_id
            grant.status = "active"
            grant.revoked_at = None
            grant.revoked_by = None
            grant.granted_by = self.user.id
            grant.granted_at = utc_now()
        self.session.flush()
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="group_skill.enabled",
            resource_type="group_skill_grant",
            resource_id=grant.id,
            after_data={
                "group_id": group_id,
                "skill_id": skill.id,
                "version_policy": data.version_policy,
            },
        )
        self.session.commit()
        return self._read(grant)

    def update(
        self,
        group_id: str,
        skill_id: str,
        data: GroupSkillGrantUpdate,
    ) -> GroupSkillGrantRead:
        self._manager(group_id)
        skill = self._published_global(skill_id)
        grant = self._grant(group_id, skill.id)
        policy = data.version_policy or grant.version_policy
        locked_version_id = (
            data.locked_version_id
            if "locked_version_id" in data.model_fields_set
            else grant.locked_version_id
        )
        self._validate_policy(skill, policy, locked_version_id)
        grant.version_policy = policy
        grant.locked_version_id = locked_version_id
        if data.status:
            grant.status = data.status
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="group_skill.updated",
            resource_type="group_skill_grant",
            resource_id=grant.id,
            after_data={"status": grant.status, "version_policy": grant.version_policy},
        )
        self.session.commit()
        return self._read(grant)

    def disable(self, group_id: str, skill_id: str) -> None:
        self._manager(group_id)
        grant = self._grant(group_id, skill_id)
        grant.status = "disabled"
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="group_skill.disabled",
            resource_type="group_skill_grant",
            resource_id=grant.id,
        )
        self.session.commit()

    def _membership(self, group_id: str) -> GroupMember:
        group = self.groups.get_active(group_id)
        membership = self.groups.membership(group_id, self.user.id) if group else None
        if group is None or membership is None:
            raise AppError("GROUP_NOT_FOUND", "Group was not found.", 404)
        return membership

    def _manager(self, group_id: str) -> None:
        membership = self._membership(group_id)
        if membership.role not in {"owner", "admin"}:
            raise AppError("PERMISSION_DENIED", "Group administrator permission is required.", 403)

    def _published_global(self, skill_id: str) -> Skill:
        skill = self.session.scalar(
            select(Skill).where(
                Skill.id == skill_id,
                Skill.skill_type == "global",
                Skill.status == "published",
            )
        )
        if skill is None:
            raise AppError("SKILL_NOT_AVAILABLE", "Global skill is not available.", 404)
        return skill

    def _grant(self, group_id: str, skill_id: str) -> GroupSkillGrant:
        grant = self.session.scalar(
            select(GroupSkillGrant).where(
                GroupSkillGrant.group_id == group_id,
                GroupSkillGrant.skill_id == skill_id,
                GroupSkillGrant.status != "revoked",
            )
        )
        if grant is None:
            raise AppError("GRANT_NOT_FOUND", "Group skill grant was not found.", 404)
        return grant

    def _validate_policy(
        self,
        skill: Skill,
        policy: str,
        locked_version_id: str | None,
    ) -> None:
        if policy == "latest":
            return
        if locked_version_id is None:
            raise AppError("LOCKED_VERSION_REQUIRED", "Locked policy requires a version.", 422)
        version = self.session.scalar(
            select(SkillVersion).where(
                SkillVersion.id == locked_version_id,
                SkillVersion.skill_id == skill.id,
                SkillVersion.status == "published",
            )
        )
        if version is None:
            raise AppError("VERSION_NOT_AVAILABLE", "Published version was not found.", 422)

    def _skill_read(self, skill: Skill) -> SkillRead:
        result = SkillRead.model_validate(skill)
        version = self.skills.version(skill.current_version_id)
        result.current_version = SkillVersionRead.model_validate(version) if version else None
        return result

    def _read(self, grant: GroupSkillGrant) -> GroupSkillGrantRead:
        result = GroupSkillGrantRead.model_validate(grant)
        skill = self.session.get(Skill, grant.skill_id)
        if skill:
            result.skill = self._skill_read(skill)
            version_id = (
                grant.locked_version_id
                if grant.version_policy == "locked"
                else skill.current_version_id
            )
            version = self.skills.version(version_id)
            result.effective_version = SkillVersionRead.model_validate(version) if version else None
        return result
