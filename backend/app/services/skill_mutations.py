from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from sqlalchemy import select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.db.base import utc_now
from app.models import Skill, SkillVersion, SyncChangeLog
from app.services.audit import write_audit
from app.services.blob_storage import BlobStorage, get_blob_storage
from app.services.legacy_package import synthesize_legacy_package


class SkillMutationService:
    """Shared transactional mutation path for Skill resources.

    This service deliberately never commits or rolls back the SQLAlchemy session.
    The caller owns the transaction boundary. Existing REST facades commit after a
    successful domain mutation; the M2 sync path will append revision/change-feed/
    receipt rows to the same transaction before committing.

    Authorization is also deliberately outside this class. A caller must resolve
    an already-authorized Skill before passing it to update/delete/version methods.

    Every domain mutation that changes the desktop-visible representation also
    appends one :class:`SyncChangeLog` row in the same transaction, so
    browser-originated and sync-originated changes surface identically in the
    M2.5 pull feed (plan §11 “Existing REST changes”).
    """

    def __init__(self, session: Session, actor_user_id: str) -> None:
        self.session = session
        self.actor_user_id = actor_user_id
        self._storage: BlobStorage | None = None

    def _package_hash_for_content(
        self,
        *,
        slug: str,
        name: str,
        description: str,
        content: Mapping[str, Any],
        package_manifest_hash: str | None,
    ) -> str | None:
        """Resolve the package identity for a new version row.

        An explicit ``package_manifest_hash`` (the sync path) wins. A legacy
        content-only write (browser REST path) synthesizes the minimal
        ``SKILL.md`` package (plan §17) so the resulting version carries an
        addressable package the desktop can pull and materialize.
        """
        if package_manifest_hash is not None:
            return package_manifest_hash
        if self._storage is None:
            self._storage = get_blob_storage(self.session)
        return synthesize_legacy_package(
            self.session,
            self._storage,
            slug=slug,
            name=name,
            description=description,
            content=dict(content),
        )

    def _emit_change_event(
        self,
        skill: Skill,
        *,
        operation: str,
        metadata: dict[str, Any] | None = None,
    ) -> None:
        payload = (
            metadata
            if metadata is not None
            else {
                "name": skill.name,
                "slug": skill.slug,
                "description": skill.description,
                "category": skill.category,
                "tags": list(skill.tags or []),
                "status": skill.status,
            }
        )
        self.session.add(
            SyncChangeLog(
                resource_type="skill",
                resource_id=skill.id,
                resource_revision=skill.sync_revision,
                operation=operation,
                owner_user_id=skill.owner_user_id,
                package_manifest_hash=skill.current_package_hash,
                metadata_payload=payload,
            )
        )

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
        package_manifest_hash: str | None = None,
        package_size_bytes: int | None = None,
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
            sync_revision=1,
            current_package_hash=package_manifest_hash,
            created_by=self.actor_user_id,
        )
        self.session.add(skill)
        self.session.flush()

        resolved_package_hash = self._package_hash_for_content(
            slug=skill.slug,
            name=skill.name,
            description=skill.description,
            content=content,
            package_manifest_hash=package_manifest_hash,
        )
        skill.current_package_hash = resolved_package_hash

        created_version = self._create_version_row(
            skill=skill,
            version=version,
            revision=skill.sync_revision,
            content=content,
            manifest=manifest,
            package_manifest_hash=resolved_package_hash,
            package_size_bytes=package_size_bytes,
            dependency_config=dependency_config,
            change_log=change_log,
            status=version_status,
        )
        skill.current_version_id = created_version.id

        after_data: dict[str, Any] = {
            "name": skill.name,
            "version": created_version.version,
            "revision": skill.sync_revision,
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
        self._emit_change_event(skill, operation="upsert")
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
        package_manifest_hash: str | None = None,
        package_size_bytes: int | None = None,
    ) -> SkillVersion | None:
        before = {
            "name": skill.name,
            "status": skill.status,
            "version": skill.current_version_id,
            "revision": skill.sync_revision,
        }

        changed = False
        for key, value in updates.items():
            if value is not None and getattr(skill, key) != value:
                setattr(skill, key, value)
                changed = True

        created_version: SkillVersion | None = None
        if content is not None:
            changed = True
            version_name = version or self.next_patch_version(skill)
            self.ensure_version_available(skill.id, version_name)
            resolved_package_hash = self._package_hash_for_content(
                slug=skill.slug,
                name=skill.name,
                description=skill.description,
                content=content,
                package_manifest_hash=package_manifest_hash,
            )
            version_manifest = (
                manifest if manifest is not None else {"name": skill.slug, "schema_version": 1}
            )
            next_revision = skill.sync_revision + 1
            created_version = self._create_version_row(
                skill=skill,
                version=version_name,
                revision=next_revision,
                content=content,
                manifest=version_manifest,
                package_manifest_hash=resolved_package_hash,
                package_size_bytes=package_size_bytes,
                dependency_config=dependency_config or {},
                change_log=change_log,
                status=version_status,
            )
            skill.current_version_id = created_version.id
            skill.current_package_hash = resolved_package_hash
            if demote_published_on_new_version and skill.status == "published":
                skill.status = "draft"

        if changed:
            skill.sync_revision += 1

        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            before_data=before,
            after_data={
                "name": skill.name,
                "status": skill.status,
                "revision": skill.sync_revision,
            },
        )
        if changed:
            self._emit_change_event(skill, operation="upsert")
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
        package_manifest_hash: str | None = None,
        package_size_bytes: int | None = None,
    ) -> SkillVersion:
        self.ensure_version_available(skill.id, version)
        next_revision = skill.sync_revision + 1
        resolved_package_hash = self._package_hash_for_content(
            slug=skill.slug,
            name=skill.name,
            description=skill.description,
            content=content,
            package_manifest_hash=package_manifest_hash,
        )
        created_version = self._create_version_row(
            skill=skill,
            version=version,
            revision=next_revision,
            content=content,
            manifest=manifest,
            package_manifest_hash=resolved_package_hash,
            package_size_bytes=package_size_bytes,
            dependency_config=dependency_config,
            change_log=change_log,
            status=version_status,
        )
        skill.current_version_id = created_version.id
        skill.current_package_hash = resolved_package_hash
        skill.sync_revision = next_revision

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
            after_data={
                "version": created_version.version,
                "status": created_version.status,
                "revision": skill.sync_revision,
            },
        )
        self._emit_change_event(skill, operation="upsert")
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

        changed = (
            version.status != "published"
            or skill.current_version_id != version.id
            or skill.status != "published"
            or skill.current_package_hash != version.package_manifest_hash
        )
        version.status = "published"
        skill.current_version_id = version.id
        skill.current_package_hash = version.package_manifest_hash
        skill.status = "published"
        if changed:
            skill.sync_revision += 1

        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            after_data={"version": version.version, "revision": skill.sync_revision},
        )
        if changed:
            self._emit_change_event(skill, operation="upsert")

    def set_status(self, skill: Skill, status: str, *, audit_action: str) -> None:
        before_status = skill.status
        if before_status != status:
            skill.status = status
            skill.sync_revision += 1
        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            before_data={"status": before_status},
            after_data={"status": status, "revision": skill.sync_revision},
        )
        if before_status != status:
            self._emit_change_event(skill, operation="upsert")

    def soft_delete(self, skill: Skill, *, audit_action: str) -> None:
        """Moves a skill into the trash (recycle bin).

        The row stays addressable by id (restore/purge need it) but every
        list feed filters ``status == 'deleted'``, and the emitted tombstone
        removes the skill from desktop mirrors.
        """
        before = {
            "name": skill.name,
            "status": skill.status,
            "revision": skill.sync_revision,
        }
        deleted = skill.status != "deleted" or skill.deleted_at is None
        if deleted:
            skill.status = "deleted"
            skill.deleted_at = utc_now()
            skill.sync_revision += 1
        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            before_data=before,
            after_data={"status": "deleted", "revision": skill.sync_revision},
        )
        if deleted:
            self._emit_change_event(skill, operation="delete")

    def restore_skill(self, skill: Skill, *, audit_action: str) -> Skill:
        """Moves a trashed skill back into the active list.

        Restored skills return as ``draft`` regardless of their previous
        status so a delete/restore round trip can never silently republish
        content. The emitted ``upsert`` event re-creates the mirror on any
        desktop that already applied the delete tombstone.
        """
        if skill.status != "deleted":
            raise AppError("SKILL_NOT_DELETED", "Only deleted skills can be restored.", 409)
        skill.status = "draft"
        skill.deleted_at = None
        skill.sync_revision += 1
        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            after_data={"status": "draft", "revision": skill.sync_revision},
        )
        self._emit_change_event(skill, operation="upsert")
        return skill

    def purge_skill(self, skill: Skill, *, audit_action: str) -> None:
        """Permanently removes a trashed skill and all of its versions.

        Only tombstoned (deleted) skills may be purged, so an accidental
        purge cannot bypass the trash. The skill row cascades to its
        versions; unreferenced blobs are reclaimed later by the existing
        mark-and-sweep GC. A second tombstone is emitted first so desktop
        mirrors that missed the soft-delete event still clean up.
        """
        if skill.status != "deleted" or skill.deleted_at is None:
            raise AppError(
                "SKILL_NOT_DELETED",
                "Only skills in the trash can be purged permanently.",
                409,
            )
        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            after_data={"purged": True, "slug": skill.slug, "name": skill.name},
        )
        self._emit_change_event(skill, operation="delete")
        self.session.delete(skill)

    def ensure_version_available(self, skill_id: str, version: str) -> None:
        exists = self.session.scalar(
            select(SkillVersion.id).where(
                SkillVersion.skill_id == skill_id,
                SkillVersion.version == version,
            )
        )
        if exists is not None:
            raise AppError("VERSION_EXISTS", "This version already exists.", 409)

    def _assert_version_tags_free(
        self,
        skill_id: str,
        tags: list[str],
        *,
        exclude_version: str,
    ) -> None:
        if not tags:
            return
        wanted = set(tags)
        rows = self.session.execute(
            select(SkillVersion.version, SkillVersion.tags).where(
                SkillVersion.skill_id == skill_id,
                SkillVersion.version != exclude_version,
            )
        ).all()
        for other_version, other_tags in rows:
            clash = wanted.intersection(other_tags or [])
            if clash:
                raise AppError(
                    "VERSION_TAG_TAKEN",
                    f"Tag(s) {sorted(clash)} already label version {other_version}.",
                    409,
                )

    def set_version_tags(
        self,
        skill: Skill,
        version: SkillVersion,
        *,
        tags: list[str],
        audit_action: str,
    ) -> SkillVersion:
        """Replaces the docker-style label set of one version.

        A tag can label only one version per skill (like a docker tag), so the
        replacement is rejected when another version already owns one of the
        requested tags. Tags are presentation metadata: the revision bump keeps
        desktop mirrors in sync but never touches packages.
        """
        if version.skill_id != skill.id:
            raise AppError("VERSION_NOT_FOUND", "Skill version was not found.", 404)
        cleaned = list(dict.fromkeys(tag.strip() for tag in tags if tag.strip()))
        self._assert_version_tags_free(skill.id, cleaned, exclude_version=version.version)

        changed = version.tags != cleaned
        version.tags = cleaned
        if changed:
            skill.sync_revision += 1
        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            after_data={
                "version": version.version,
                "tags": cleaned,
                "revision": skill.sync_revision,
            },
        )
        if changed:
            self._emit_change_event(skill, operation="upsert")
        return version

    def rollback_version(
        self,
        skill: Skill,
        source: SkillVersion,
        *,
        change_log: str,
        audit_action: str,
    ) -> SkillVersion:
        """Creates a new version carrying the source version's content.

        Rollback is additive (like a re-mint), not a history rewrite: a new
        semantic version carries the old payload and becomes the skill's
        current version, so desktop pulls receive it as a normal forward
        change and the audit trail stays append-only.
        """
        if source.skill_id != skill.id:
            raise AppError("VERSION_NOT_FOUND", "Skill version was not found.", 404)
        version_name = self.next_patch_version(skill)
        self.ensure_version_available(skill.id, version_name)

        note = change_log or f"回滚自 {source.version}"
        created = self._create_version_row(
            skill=skill,
            version=version_name,
            revision=skill.sync_revision + 1,
            content=dict(source.content),
            manifest=dict(source.manifest) or {"name": skill.slug, "schema_version": 1},
            package_manifest_hash=source.package_manifest_hash,
            package_size_bytes=source.package_size_bytes,
            dependency_config=dict(source.dependency_config or {}),
            change_log=note,
            status="draft",
        )
        skill.current_version_id = created.id
        skill.current_package_hash = created.package_manifest_hash
        skill.sync_revision += 1
        if skill.status == "published":
            skill.status = "draft"

        write_audit(
            self.session,
            actor_user_id=self.actor_user_id,
            action=audit_action,
            resource_type="skill",
            resource_id=skill.id,
            after_data={
                "rollback_from": source.version,
                "version": created.version,
                "revision": skill.sync_revision,
            },
        )
        self._emit_change_event(skill, operation="upsert")
        return created

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
        revision: int,
        content: Mapping[str, Any],
        manifest: Mapping[str, Any],
        package_manifest_hash: str | None,
        package_size_bytes: int | None,
        dependency_config: Mapping[str, Any],
        change_log: str,
        status: str,
    ) -> SkillVersion:
        created_version = SkillVersion(
            skill_id=skill.id,
            version=version,
            revision=revision,
            content=dict(content),
            manifest=dict(manifest),
            package_manifest_hash=package_manifest_hash,
            package_size_bytes=package_size_bytes,
            dependency_config=dict(dependency_config),
            change_log=change_log,
            status=status,
            created_by=self.actor_user_id,
        )
        self.session.add(created_version)
        self.session.flush()
        return created_version
