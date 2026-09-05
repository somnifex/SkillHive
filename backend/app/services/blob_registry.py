"""M2.2 package blob registry.

Bridges the physical :class:`BlobStorage` backends and the
``skill_blob_objects`` metadata table. Blob rows are only created after the
bytes have been durably stored and verified, so a row can be trusted as
"addressable object exists" by the mutation path.
"""

from __future__ import annotations

from datetime import UTC, datetime

from sqlalchemy import select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.models import SkillBlobObject
from app.schemas.sync import MAX_BLOB_BYTES, SyncBlobDescriptor
from app.services.blob_storage import BlobStorage, BlobStorageError, is_canonical_hash


def missing_blobs(
    session: Session,
    storage: BlobStorage,
    descriptors: list[SyncBlobDescriptor],
) -> list[SyncBlobDescriptor]:
    """Return the subset of descriptors the server does not yet hold.

    An object counts as present only when both the metadata row and the
    physical bytes exist and verify. A row without bytes (metadata/bytes
    divergence) is reported missing so the client re-uploads it.
    """
    hashes = [descriptor.hash for descriptor in descriptors]
    rows = {
        row.hash: row
        for row in session.scalars(select(SkillBlobObject).where(SkillBlobObject.hash.in_(hashes)))
    }

    missing: list[SyncBlobDescriptor] = []
    for descriptor in descriptors:
        row = rows.get(descriptor.hash)
        if row is None or not storage.exists(descriptor.hash, descriptor.size_bytes):
            missing.append(descriptor)
    return missing


def register_verified_blob(
    session: Session,
    storage: BlobStorage,
    hash_value: str,
    payload: bytes,
) -> SkillBlobObject:
    """Store payload bytes then record the verified metadata row.

    The physical write happens first and fully verifies the digest; the row is
    inserted afterwards within the caller's transaction so that a committed row
    always implies addressable, verified bytes.
    """
    if not is_canonical_hash(hash_value):
        raise AppError("BLOB_INVALID_HASH", "Blob hash is not canonical.", 400)
    if len(payload) > MAX_BLOB_BYTES:
        raise AppError("BLOB_TOO_LARGE", "Blob exceeds the per-object size limit.", 413)

    try:
        storage.put_verified(hash_value, payload)
    except BlobStorageError as error:
        raise AppError(error.code, error.message, error.status_code, error.details) from error

    existing = session.get(SkillBlobObject, hash_value)
    if existing is not None:
        return existing

    row = SkillBlobObject(
        hash=hash_value,
        size_bytes=len(payload),
        storage_key=hash_value,
        storage_backend=storage.backend_name,
        verified_at=datetime.now(UTC),
    )
    session.add(row)
    session.flush()
    return row
