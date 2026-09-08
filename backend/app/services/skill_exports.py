from __future__ import annotations

import io
import zipfile

from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.models import Skill, SkillVersion
from app.services.blob_storage import get_blob_storage
from app.services.legacy_package import synthesize_legacy_package
from app.services.package_manifest import validate_snapshot_manifest_bytes

MAX_EXPORT_BYTES = 512 * 1024 * 1024


def export_version_zip(session: Session, skill: Skill, version: SkillVersion) -> bytes:
    """Build a portable zip from a verified Skill version package."""
    storage = get_blob_storage(session)
    manifest_hash = version.package_manifest_hash
    if manifest_hash is None:
        manifest_hash = synthesize_legacy_package(
            session,
            storage,
            slug=skill.slug,
            name=skill.name,
            description=skill.description,
            content=dict(version.content),
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
