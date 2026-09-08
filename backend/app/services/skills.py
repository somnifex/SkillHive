from math import ceil

from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.db.session import begin_sqlite_immediate_write
from app.models import Skill, User
from app.repositories.skills import SkillRepository
from app.schemas.common import Page
from app.schemas.skill import (
    SkillCreate,
    SkillRead,
    SkillUpdate,
    SkillVersionCreate,
    SkillVersionRead,
    VersionRollbackRequest,
)
from app.services.blob_storage import get_blob_storage
from app.services.legacy_package import synthesize_legacy_package
from app.services.package_manifest import validate_snapshot_manifest_bytes
from app.services.skill_mutations import SkillMutationService

# Hard ceiling for in-memory version export; matches the sync package cap.
MAX_EXPORT_BYTES = 512 * 1024 * 1024


class PrivateSkillService:
    """Authorization/read facade for private Skills.

    REST transaction ownership remains here for backward-compatible behavior.
    The actual Skill mutations are delegated to SkillMutationService, which does
    not commit and can therefore be reused by the M2 sync transaction boundary.
    """

    def __init__(self, session: Session, user: User) -> None:
        self.session = session
        self.user = user
        self.repository = SkillRepository(session)
        self.mutations = SkillMutationService(session, user.id)

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
        self._begin_private_write()
        if self.repository.slug_exists(self.user.id, data.slug):
            raise AppError("SKILL_SLUG_TAKEN", "A skill with this slug already exists.", 409)

        skill, _ = self.mutations.create_skill(
            name=data.name,
            slug=data.slug,
            description=data.description,
            skill_type="private",
            owner_user_id=self.user.id,
            category=data.category,
            tags=data.tags,
            skill_status="draft",
            version=data.version,
            content=data.content.model_dump(mode="json"),
            manifest={"name": data.slug, "schema_version": 1},
            dependency_config={},
            change_log=data.change_log,
            version_status="draft",
            audit_action="private_skill.created",
        )
        self.session.commit()
        return self._read(skill)

    def get(self, skill_id: str) -> SkillRead:
        return self._read(self._owned(skill_id))

    def update(self, skill_id: str, data: SkillUpdate) -> SkillRead:
        self._begin_private_write()
        skill = self._owned_for_write(skill_id)
        updates = data.model_dump(exclude_unset=True, exclude={"content", "version", "change_log"})
        content = data.content.model_dump(mode="json") if data.content is not None else None
        self.mutations.update_skill(
            skill,
            updates=updates,
            audit_action="private_skill.updated",
            content=content,
            version=data.version,
            change_log=data.change_log,
            version_status="draft",
        )
        self.session.commit()
        return self._read(skill)

    def create_version(self, skill_id: str, data: SkillVersionCreate) -> SkillVersionRead:
        self._begin_private_write()
        skill = self._owned_for_write(skill_id)
        version = self.mutations.create_version(
            skill,
            version=data.version,
            content=data.content.model_dump(mode="json"),
            manifest=data.manifest,
            dependency_config=data.dependency_config,
            change_log=data.change_log,
            version_status=data.status,
            audit_action="private_skill.version_created",
            publish_skill=data.status == "published",
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

    def version_by_name(self, skill_id: str, version: str) -> SkillVersionRead:
        skill = self._owned(skill_id)
        for row in self.repository.versions(skill.id):
            if row.version == version:
                return SkillVersionRead.model_validate(row)
        raise AppError("VERSION_NOT_FOUND", "Skill version was not found.", 404)

    def set_version_tags(self, skill_id: str, version: str, tags: list[str]) -> SkillVersionRead:
        self._begin_private_write()
        skill = self._owned_for_write(skill_id)
        row = self.repository.version_by_name_for_update(skill.id, version)
        if row is None:
            raise AppError("VERSION_NOT_FOUND", "Skill version was not found.", 404)
        updated = self.mutations.set_version_tags(
            skill, row, tags=tags, audit_action="private_skill.version_tagged"
        )
        self.session.commit()
        return SkillVersionRead.model_validate(updated)

    def rollback(self, skill_id: str, data: VersionRollbackRequest) -> SkillVersionRead:
        self._begin_private_write()
        skill = self._owned_for_write(skill_id)
        row = self.repository.version_by_name_for_update(skill.id, data.version)
        if row is None:
            raise AppError("VERSION_NOT_FOUND", "Skill version was not found.", 404)
        created = self.mutations.rollback_version(
            skill,
            row,
            change_log=data.change_log,
            audit_action="private_skill.version_rolled_back",
        )
        self.session.commit()
        return SkillVersionRead.model_validate(created)

    def delete(self, skill_id: str) -> None:
        self._begin_private_write()
        skill = self._owned_for_write(skill_id)
        self.mutations.soft_delete(skill, audit_action="private_skill.deleted")
        self.session.commit()

    def trash_page(
        self,
        *,
        page: int,
        page_size: int,
        query: str | None,
    ) -> Page[SkillRead]:
        items, total = self.repository.list_private_trash(
            self.user.id, page=page, page_size=page_size, query=query
        )
        return Page[SkillRead](
            items=[self._read(skill, include_content=False) for skill in items],
            page=page,
            page_size=page_size,
            total=total,
            pages=ceil(total / page_size) if total else 0,
        )

    def restore(self, skill_id: str) -> SkillRead:
        self._begin_private_write()
        skill = self.repository.private_trashed_for_owner_for_update(skill_id, self.user.id)
        if skill is None:
            raise AppError("SKILL_NOT_FOUND", "Skill was not found in the trash.", 404)
        restored = self.mutations.restore_skill(skill, audit_action="private_skill.restored")
        self.session.commit()
        return self._read(restored)

    def purge(self, skill_id: str) -> None:
        self._begin_private_write()
        skill = self.repository.private_trashed_for_owner_for_update(skill_id, self.user.id)
        if skill is None:
            raise AppError("SKILL_NOT_FOUND", "Skill was not found in the trash.", 404)
        self.mutations.purge_skill(skill, audit_action="private_skill.purged")
        self.session.commit()

    def export_version_zip(self, skill_id: str, version: str) -> bytes:
        """Builds a portable zip archive of one historical version.

        Real packages are re-materialized from the content-addressed blob
        store after manifest re-validation; legacy content-only versions
        synthesize their minimal SKILL.md package exactly like the sync pull
        path, so the archive matches what a desktop would deploy.
        """
        import io
        import zipfile

        skill = self._owned(skill_id)
        row = self.repository.version_by_name(skill.id, version)
        if row is None:
            raise AppError("VERSION_NOT_FOUND", "Skill version was not found.", 404)

        storage = get_blob_storage(self.session)
        manifest_hash = row.package_manifest_hash
        if manifest_hash is None:
            manifest_hash = synthesize_legacy_package(
                self.session,
                storage,
                slug=skill.slug,
                name=skill.name,
                description=skill.description,
                content=dict(row.content),
            )
        if manifest_hash is None:
            raise AppError(
                "VERSION_NOT_EXPORTABLE",
                "This version has no package content to export.",
                409,
            )

        manifest_bytes = storage.open(manifest_hash).read()
        files = validate_snapshot_manifest_bytes(manifest_bytes)
        entries: list[tuple[str, bytes]] = []
        total = 0
        for entry in files:
            payload = storage.open(entry["blob_hash"]).read()
            if len(payload) != entry["size_bytes"]:
                raise AppError(
                    "BLOB_SIZE_MISMATCH",
                    "Stored blob size does not match the manifest.",
                    500,
                )
            total += len(payload)
            if total > MAX_EXPORT_BYTES:
                raise AppError(
                    "EXPORT_TOO_LARGE",
                    "This version exceeds the export size limit.",
                    413,
                )
            entries.append((entry["path"], payload))

        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
            for path, payload in entries:
                archive.writestr(path, payload)
        return buffer.getvalue()

    def categories(self) -> list[str]:
        return self.repository.private_categories(self.user.id)

    def _owned(self, skill_id: str) -> Skill:
        skill = self.repository.private_for_owner(skill_id, self.user.id)
        if skill is None:
            raise AppError("SKILL_NOT_FOUND", "Skill was not found.", 404)
        return skill

    def _owned_for_write(self, skill_id: str) -> Skill:
        skill = self.repository.private_for_owner_for_update(skill_id, self.user.id)
        if skill is None:
            raise AppError("SKILL_NOT_FOUND", "Skill was not found.", 404)
        return skill

    def _begin_private_write(self) -> None:
        """Acquire a database-level write boundary before reading the head.

        PostgreSQL uses ``FOR UPDATE`` in the repository query. SQLite does
        not implement row locks, so an immediate transaction serializes the
        read/modify/flush sequence against other SQLite writers. The session
        may still contain the read-only transaction opened by auth dependency
        queries; rolling that back is safe because this facade owns all
        mutations for the request and the authenticated User object is
        re-used only by identity.
        """
        begin_sqlite_immediate_write(self.session)

    def _read(self, skill: Skill, *, include_content: bool = True) -> SkillRead:
        current = self.repository.version(skill.current_version_id) if include_content else None
        result = SkillRead.model_validate(skill)
        result.current_version = SkillVersionRead.model_validate(current) if current else None
        return result
