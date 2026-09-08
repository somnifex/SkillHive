from __future__ import annotations

from math import ceil

from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.db.session import begin_sqlite_immediate_write
from app.models import Group, Skill, User
from app.repositories.groups import GroupRepository
from app.repositories.skills import SkillRepository
from app.schemas.common import Page
from app.schemas.skill import (
    PublishToGroupRequest,
    SkillCreate,
    SkillRead,
    SkillUpdate,
    SkillVersionCreate,
    SkillVersionRead,
    VersionRollbackRequest,
)
from app.services.skill_exports import export_version_zip
from app.services.skill_mutations import SkillMutationService

MANAGER_ROLES = frozenset({"owner", "admin"})


class GroupOwnedSkillService:
    """Authorization facade for Skills owned by one group.

    A group Skill is a separate resource from the author's personal Skill.
    ``created_by`` is retained as the author, while ``group_id`` defines the
    ownership scope. Effective group managers and the author can mutate it;
    every caller must still have an active relationship with the group.
    """

    def __init__(self, session: Session, user: User) -> None:
        self.session = session
        self.user = user
        self.groups = GroupRepository(session)
        self.skills = SkillRepository(session)
        self.mutations = SkillMutationService(session, user.id)

    def list_page(
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
    ) -> Page[SkillRead]:
        _group, role = self._group_context(group_id)
        items, total = self.skills.list_group(
            group_id,
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
            items=[
                self._read(skill, include_content=False, can_manage=self._can_manage(role, skill))
                for skill in items
            ],
            page=page,
            page_size=page_size,
            total=total,
            pages=ceil(total / page_size) if total else 0,
        )

    def get(self, group_id: str, skill_id: str) -> SkillRead:
        _group, role = self._group_context(group_id)
        skill = self._active_skill(skill_id, group_id)
        return self._read(skill, can_manage=self._can_manage(role, skill))

    def trash_page(
        self,
        group_id: str,
        *,
        page: int,
        page_size: int,
        query: str | None,
    ) -> Page[SkillRead]:
        _group, role = self._group_context(group_id)
        items, total = self.skills.list_group_trash(
            group_id,
            page=page,
            page_size=page_size,
            query=query,
        )
        return Page[SkillRead](
            items=[
                self._read(skill, include_content=False, can_manage=self._can_manage(role, skill))
                for skill in items
            ],
            page=page,
            page_size=page_size,
            total=total,
            pages=ceil(total / page_size) if total else 0,
        )

    def create(self, group_id: str, data: SkillCreate) -> SkillRead:
        self._begin_write()
        self._group_context(group_id)
        if self.skills.group_slug_exists(group_id, data.slug):
            raise AppError("SKILL_SLUG_TAKEN", "A group Skill with this slug already exists.", 409)

        skill, _ = self.mutations.create_skill(
            name=data.name,
            slug=data.slug,
            description=data.description,
            skill_type="group",
            owner_user_id=None,
            group_id=group_id,
            category=data.category,
            tags=data.tags,
            skill_status="published",
            version=data.version,
            content=data.content.model_dump(mode="json"),
            manifest={"name": data.slug, "schema_version": 1},
            dependency_config={},
            change_log=data.change_log,
            version_status="published",
            audit_action="group_skill.created",
            audit_after_data={"group_id": group_id, "author_user_id": self.user.id},
        )
        self.session.commit()
        return self._read(skill, can_manage=True)

    def publish_personal(
        self,
        skill_id: str,
        data: PublishToGroupRequest,
    ) -> SkillRead:
        self._begin_write()
        group, role = self._group_context(data.group_id)
        source = self.skills.private_for_owner(skill_id, self.user.id)
        if source is None:
            raise AppError("SKILL_NOT_FOUND", "Skill was not found.", 404)

        source_version = self.skills.version(source.current_version_id)
        slug = self._available_slug(group.id, source.slug)
        content = source_version.content if source_version else {}
        skill, _ = self.mutations.create_skill(
            name=source.name,
            slug=slug,
            description=source.description,
            skill_type="group",
            owner_user_id=None,
            group_id=group.id,
            category=source.category,
            tags=list(source.tags or []),
            skill_status="published",
            version=source_version.version if source_version else "0.1.0",
            content=content,
            manifest=(
                dict(source_version.manifest)
                if source_version and source_version.manifest
                else {"name": slug, "schema_version": 1}
            ),
            dependency_config=(
                dict(source_version.dependency_config or {}) if source_version else {}
            ),
            change_log=f"从个人 Skill「{source.name}」发布到群组",
            version_status="published",
            audit_action="group_skill.published_from_personal",
            audit_after_data={
                "group_id": group.id,
                "source_skill_id": source.id,
                "author_user_id": self.user.id,
            },
            package_manifest_hash=source_version.package_manifest_hash if source_version else None,
            package_size_bytes=source_version.package_size_bytes if source_version else None,
        )
        self.session.commit()
        return self._read(skill, can_manage=self._can_manage(role, skill))

    def update(self, group_id: str, skill_id: str, data: SkillUpdate) -> SkillRead:
        self._begin_write()
        _group, role = self._group_context(group_id)
        skill = self._writable_skill(group_id, skill_id, role)
        updates = data.model_dump(exclude_unset=True, exclude={"content", "version", "change_log"})
        content = data.content.model_dump(mode="json") if data.content is not None else None
        self.mutations.update_skill(
            skill,
            updates=updates,
            audit_action="group_skill.updated",
            content=content,
            version=data.version,
            change_log=data.change_log,
            version_status="draft",
        )
        self.session.commit()
        return self._read(skill, can_manage=True)

    def create_version(
        self, group_id: str, skill_id: str, data: SkillVersionCreate
    ) -> SkillVersionRead:
        self._begin_write()
        _group, role = self._group_context(group_id)
        skill = self._writable_skill(group_id, skill_id, role)
        version = self.mutations.create_version(
            skill,
            version=data.version,
            content=data.content.model_dump(mode="json"),
            manifest=data.manifest,
            dependency_config=data.dependency_config,
            change_log=data.change_log,
            version_status=data.status,
            audit_action="group_skill.version_created",
            publish_skill=data.status == "published",
        )
        self.session.commit()
        return SkillVersionRead.model_validate(version)

    def versions(self, group_id: str, skill_id: str) -> list[SkillVersionRead]:
        self._group_context(group_id)
        skill = self._active_skill(skill_id, group_id)
        return [
            SkillVersionRead.model_validate(version) for version in self.skills.versions(skill.id)
        ]

    def set_version_tags(
        self, group_id: str, skill_id: str, version: str, tags: list[str]
    ) -> SkillVersionRead:
        self._begin_write()
        _group, role = self._group_context(group_id)
        skill = self._writable_skill(group_id, skill_id, role)
        row = self.skills.version_by_name_for_update(skill.id, version)
        if row is None:
            raise AppError("VERSION_NOT_FOUND", "Skill version was not found.", 404)
        updated = self.mutations.set_version_tags(
            skill, row, tags=tags, audit_action="group_skill.version_tagged"
        )
        self.session.commit()
        return SkillVersionRead.model_validate(updated)

    def rollback(
        self, group_id: str, skill_id: str, data: VersionRollbackRequest
    ) -> SkillVersionRead:
        self._begin_write()
        _group, role = self._group_context(group_id)
        skill = self._writable_skill(group_id, skill_id, role)
        row = self.skills.version_by_name_for_update(skill.id, data.version)
        if row is None:
            raise AppError("VERSION_NOT_FOUND", "Skill version was not found.", 404)
        created = self.mutations.rollback_version(
            skill,
            row,
            change_log=data.change_log,
            audit_action="group_skill.version_rolled_back",
        )
        self.session.commit()
        return SkillVersionRead.model_validate(created)

    def delete(self, group_id: str, skill_id: str) -> None:
        self._begin_write()
        _group, role = self._group_context(group_id)
        skill = self._writable_skill(group_id, skill_id, role)
        self.mutations.soft_delete(skill, audit_action="group_skill.deleted")
        self.session.commit()

    def restore(self, group_id: str, skill_id: str) -> SkillRead:
        self._begin_write()
        _group, role = self._group_context(group_id)
        skill = self.skills.group_trashed_for_update(skill_id, group_id)
        if skill is None:
            raise AppError("SKILL_NOT_FOUND", "Skill was not found in the trash.", 404)
        self._assert_can_manage(role, skill)
        restored = self.mutations.restore_skill(skill, audit_action="group_skill.restored")
        self.session.commit()
        return self._read(restored, can_manage=True)

    def purge(self, group_id: str, skill_id: str) -> None:
        self._begin_write()
        _group, role = self._group_context(group_id)
        skill = self.skills.group_trashed_for_update(skill_id, group_id)
        if skill is None:
            raise AppError("SKILL_NOT_FOUND", "Skill was not found in the trash.", 404)
        self._assert_can_manage(role, skill)
        self.mutations.purge_skill(skill, audit_action="group_skill.purged")
        self.session.commit()

    def export_version_zip(self, group_id: str, skill_id: str, version: str) -> bytes:
        self._group_context(group_id)
        skill = self._active_skill(skill_id, group_id)
        row = self.skills.version_by_name(skill.id, version)
        if row is None:
            raise AppError("VERSION_NOT_FOUND", "Skill version was not found.", 404)
        return export_version_zip(self.session, skill, row)

    def _group_context(self, group_id: str) -> tuple[Group, str]:
        group = self.groups.get_active(group_id)
        if group is None:
            raise AppError("GROUP_NOT_FOUND", "Group was not found.", 404)
        if self.user.is_global_admin:
            return group, "owner"
        role = self.groups.effective_roles([group_id], self.user.id).get(group_id)
        if role is None:
            raise AppError("GROUP_NOT_FOUND", "Group was not found.", 404)
        return group, role

    def _active_skill(self, skill_id: str, group_id: str) -> Skill:
        skill = self.skills.group_for_member(skill_id, group_id)
        if skill is None:
            raise AppError("SKILL_NOT_FOUND", "Group Skill was not found.", 404)
        return skill

    def _writable_skill(self, group_id: str, skill_id: str, role: str) -> Skill:
        skill = self.skills.group_for_member_for_update(skill_id, group_id)
        if skill is None:
            raise AppError("SKILL_NOT_FOUND", "Group Skill was not found.", 404)
        self._assert_can_manage(role, skill)
        return skill

    def _assert_can_manage(self, role: str, skill: Skill) -> None:
        if role not in MANAGER_ROLES and skill.created_by != self.user.id:
            raise AppError(
                "PERMISSION_DENIED",
                "Only group administrators or the Skill author can manage this Skill.",
                403,
            )

    def _can_manage(self, role: str, skill: Skill) -> bool:
        return role in MANAGER_ROLES or skill.created_by == self.user.id

    def _read(
        self,
        skill: Skill,
        *,
        can_manage: bool,
        include_content: bool = True,
    ) -> SkillRead:
        result = SkillRead.model_validate(skill)
        result.can_manage = can_manage
        if include_content:
            current = self.skills.version(skill.current_version_id)
            result.current_version = SkillVersionRead.model_validate(current) if current else None
        return result

    def _available_slug(self, group_id: str, source_slug: str) -> str:
        if not self.skills.group_slug_exists(group_id, source_slug):
            return source_slug
        base = f"{source_slug[:132].rstrip('-')}-shared"
        slug = base
        suffix = 2
        while self.skills.group_slug_exists(group_id, slug):
            suffix_text = f"-{suffix}"
            slug = f"{base[: 140 - len(suffix_text)]}{suffix_text}"
            suffix += 1
        return slug

    def _begin_write(self) -> None:
        begin_sqlite_immediate_write(self.session)
