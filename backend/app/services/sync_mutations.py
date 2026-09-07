"""M2.4 idempotent mutation push service.

Implements the receipt-keyed idempotency transaction pattern from
``docs/architecture/m2-cloud-sync-plan.md`` §9:

1. authenticate user and validate the active device;
2. query receipt by ``(user, device, mutation)`` — a committed receipt is
   replayed without re-running the mutation;
3. lock the target Skill row (update/delete);
4. enforce ``base_revision`` optimistic concurrency (no LWW);
5. verify package closure before create/update commit;
6. apply the domain mutation through :class:`SkillMutationService`;
7. append change event + audit + mutation receipt in the same transaction;
8. return the result for the endpoint's commit.

Definitive logical outcomes are receipt-backed. Transport/infrastructure
failures (missing blobs, storage errors) raise before any receipt is written
so the desktop can retry the same mutation ID safely.
"""

from __future__ import annotations

from typing import Any

from sqlalchemy import select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.db.base import utc_now
from app.models import Device, Skill, SkillBlobObject, SyncMutationReceipt
from app.schemas.sync import SyncMutationRequest
from app.services.blob_storage import BlobStorage
from app.services.package_manifest import (
    SKILL_ENTRYPOINT,
    validate_package_closure,
    validate_snapshot_manifest_bytes,
)
from app.services.skill_mutations import SkillMutationService

OPERATION_CREATE = "create"
OPERATION_UPDATE = "update"
OPERATION_DELETE = "delete"


class SyncMutationError(AppError):
    """Mutation-level failure with a definitive protocol mapping."""


def validate_active_device(session: Session, user_id: str, device_id: str) -> Device:
    """Resolve and validate the caller's device; refresh last_seen_at."""
    device = session.get(Device, device_id)
    if device is None or device.user_id != user_id:
        raise SyncMutationError("DEVICE_NOT_FOUND", "Device was not found for this account.", 404)
    if device.revoked_at is not None:
        raise SyncMutationError(
            "DEVICE_REVOKED",
            "This device has been revoked; synchronization is not permitted.",
            403,
        )
    device.last_seen_at = utc_now()
    return device


def _skill_metadata(skill: Skill) -> dict[str, Any]:
    return {
        "name": skill.name,
        "slug": skill.slug,
        "description": skill.description,
        "category": skill.category,
        "tags": list(skill.tags or []),
    }


def _conflict_head(skill: Skill) -> dict[str, Any]:
    return {
        "remoteSkillId": skill.id,
        "revision": skill.sync_revision,
        "packageManifestHash": skill.current_package_hash,
        "metadata": _skill_metadata(skill),
    }


def _verify_package_closure(
    session: Session, storage: BlobStorage, manifest_hash: str
) -> tuple[int, int]:
    """Validate the stored manifest and verify every referenced blob exists."""
    row = session.get(SkillBlobObject, manifest_hash)
    if row is None:
        raise AppError(
            "BLOB_MISSING",
            "Package references a blob the server does not hold.",
            409,
            {"hash": manifest_hash},
        )

    with storage.open(manifest_hash) as stream:
        manifest_bytes = stream.read()
    files = validate_snapshot_manifest_bytes(manifest_bytes)
    return validate_package_closure(
        files,
        lambda hash_value, size_bytes: _blob_present(session, storage, hash_value, size_bytes),
    )


def _blob_present(session: Session, storage: BlobStorage, hash_value: str, size_bytes: int) -> bool:
    row = session.get(SkillBlobObject, hash_value)
    return row is not None and storage.exists(hash_value, size_bytes)


def _legacy_entrypoint_markdown(session: Session, storage: BlobStorage, manifest_hash: str) -> str:
    """Synthesize legacy ``skill_markdown`` from the package's SKILL.md entrypoint.

    The current web UI renders ``content.skill_markdown``; a sync-created
    version therefore populates it from the package entrypoint (plan §17).
    Missing or unreadable entrypoints degrade to an empty legacy body — the
    manifest remains the canonical desktop representation. Callers must have
    verified package closure first, so an absent manifest row is a 500-grade
    invariant violation rather than a client error.
    """
    row = session.get(SkillBlobObject, manifest_hash)
    if row is None:
        raise AppError(
            "BLOB_MISSING",
            "Package references a blob the server does not hold.",
            409,
            {"hash": manifest_hash},
        )
    with storage.open(manifest_hash) as stream:
        manifest_bytes = stream.read()
    try:
        files = validate_snapshot_manifest_bytes(manifest_bytes)
    except AppError:
        return ""
    entry = next((item for item in files if item["path"] == SKILL_ENTRYPOINT), None)
    if entry is None:
        return ""
    try:
        with storage.open(entry["blob_hash"]) as stream:
            return stream.read().decode("utf-8", errors="replace")
    except AppError:
        return ""


def _insert_receipt(
    session: Session,
    *,
    user_id: str,
    device_id: str,
    request: SyncMutationRequest,
    result_code: str,
    result_revision: int | None,
    resource_id: str | None,
    response_payload: dict[str, Any],
) -> SyncMutationReceipt:
    receipt = SyncMutationReceipt(
        user_id=user_id,
        device_id=device_id,
        mutation_id=str(request.mutation_id),
        operation=request.operation,
        resource_type="skill",
        resource_id=resource_id,
        result_code=result_code,
        result_revision=result_revision,
        response_payload=response_payload,
    )
    session.add(receipt)
    session.flush()
    return receipt


def _load_skill_locked(session: Session, skill_id: str, user_id: str) -> Skill | None:
    """Load the caller's private Skill with a row lock held until commit."""
    skill = session.scalar(select(Skill).where(Skill.id == skill_id).with_for_update())
    if skill is None or skill.owner_user_id != user_id or skill.skill_type != "private":
        return None
    return skill


def _receipt_response(receipt: SyncMutationReceipt) -> dict[str, Any]:
    return dict(receipt.response_payload)


def _acked_payload(
    mutation_id: str, remote_skill_id: str, revision: int, manifest_hash: str | None
) -> dict[str, Any]:
    return {
        "mutationId": mutation_id,
        "status": "acked",
        "result": {
            "remoteSkillId": remote_skill_id,
            "revision": revision,
            "packageManifestHash": manifest_hash,
        },
    }


def _error_payload(
    mutation_id: str,
    status: str,
    error_code: str,
    message: str,
    *,
    conflict: dict[str, Any] | None = None,
    retryable: bool = False,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "mutationId": mutation_id,
        "status": status,
        "errorCode": error_code,
        "message": message[:1000],
        "retryable": retryable,
    }
    if conflict is not None:
        payload["conflict"] = conflict
    return payload


def _find_skill_by_slug(session: Session, user_id: str, slug: str) -> Skill | None:
    return session.scalar(
        select(Skill).where(
            Skill.owner_user_id == user_id,
            Skill.slug == slug,
            Skill.skill_type == "private",
            Skill.status != "deleted",
        )
    )


def _handle_create(
    session: Session,
    *,
    user_id: str,
    device: Device,
    request: SyncMutationRequest,
    storage: BlobStorage,
) -> dict[str, Any]:
    assert request.metadata is not None and request.package_manifest_hash is not None
    mutation_id = str(request.mutation_id)

    if _find_skill_by_slug(session, user_id, request.metadata.slug) is not None:
        receipt = _insert_receipt(
            session,
            user_id=user_id,
            device_id=device.id,
            request=request,
            result_code="validation_error",
            result_revision=None,
            resource_id=None,
            response_payload=_error_payload(
                mutation_id,
                "validation_error",
                "SKILL_SLUG_TAKEN",
                "A skill with this slug already exists.",
            ),
        )
        return _receipt_response(receipt)

    # Closure verification happens before any domain mutation; missing blobs
    # raise AppError and leave no receipt so the desktop uploads and retries.
    _verify_package_closure(session, storage, request.package_manifest_hash)

    entrypoint_markdown = _legacy_entrypoint_markdown(
        session, storage, request.package_manifest_hash
    )
    mutations = SkillMutationService(session, user_id)
    skill, _version = mutations.create_skill(
        name=request.metadata.name,
        slug=request.metadata.slug,
        description=request.metadata.description,
        skill_type="private",
        owner_user_id=user_id,
        category=request.metadata.category,
        tags=request.metadata.tags,
        skill_status="draft",
        version="0.1.0",
        content={
            "schema_version": 1,
            "skill_markdown": entrypoint_markdown,
            # Plan §17: sync writes populate enough legacy content for the
            # current web UI to keep displaying and editing the Skill. The
            # web editor reads/writes `instructions`; mirroring the
            # entrypoint there keeps a browser edit from seeing the body as
            # empty. The package manifest stays canonical for the desktop.
            "instructions": entrypoint_markdown,
        },
        manifest={"name": request.metadata.slug, "schema_version": 1},
        dependency_config={},
        change_log="Created from desktop sync",
        version_status="draft",
        audit_action="sync_skill.created",
        package_manifest_hash=request.package_manifest_hash,
    )
    receipt = _insert_receipt(
        session,
        user_id=user_id,
        device_id=device.id,
        request=request,
        result_code="acked",
        result_revision=skill.sync_revision,
        resource_id=skill.id,
        response_payload=_acked_payload(
            mutation_id, skill.id, skill.sync_revision, skill.current_package_hash
        ),
    )
    return _receipt_response(receipt)


def _handle_update(
    session: Session,
    *,
    user_id: str,
    device: Device,
    request: SyncMutationRequest,
    storage: BlobStorage,
) -> dict[str, Any]:
    assert request.remote_skill_id is not None
    assert request.metadata is not None and request.package_manifest_hash is not None
    mutation_id = str(request.mutation_id)
    base_revision = request.base_revision or 0

    skill = _load_skill_locked(session, request.remote_skill_id, user_id)
    if skill is None:
        receipt = _insert_receipt(
            session,
            user_id=user_id,
            device_id=device.id,
            request=request,
            result_code="permission_denied",
            result_revision=None,
            resource_id=request.remote_skill_id,
            response_payload=_error_payload(
                mutation_id,
                "permission_denied",
                "SKILL_NOT_FOUND",
                "Skill was not found for this account.",
            ),
        )
        return _receipt_response(receipt)

    if skill.status == "deleted":
        receipt = _insert_receipt(
            session,
            user_id=user_id,
            device_id=device.id,
            request=request,
            result_code="validation_error",
            result_revision=None,
            resource_id=skill.id,
            response_payload=_error_payload(
                mutation_id,
                "validation_error",
                "SKILL_DELETED",
                "This skill was deleted on the server.",
            ),
        )
        return _receipt_response(receipt)

    if skill.sync_revision != base_revision:
        receipt = _insert_receipt(
            session,
            user_id=user_id,
            device_id=device.id,
            request=request,
            result_code="conflict",
            result_revision=None,
            resource_id=skill.id,
            response_payload=_error_payload(
                mutation_id,
                "conflict",
                "REVISION_CONFLICT",
                "The server head has advanced past baseRevision.",
                conflict=_conflict_head(skill),
            ),
        )
        return _receipt_response(receipt)

    _verify_package_closure(session, storage, request.package_manifest_hash)

    metadata = request.metadata
    entrypoint_markdown = _legacy_entrypoint_markdown(
        session, storage, request.package_manifest_hash
    )
    mutations = SkillMutationService(session, user_id)
    created_version = mutations.update_skill(
        skill,
        updates={
            "name": metadata.name,
            "description": metadata.description,
            "category": metadata.category,
            "tags": metadata.tags,
        },
        audit_action="sync_skill.updated",
        content={
            "schema_version": 1,
            "skill_markdown": entrypoint_markdown,
            # Mirrors the create path: keep the legacy `instructions` body in
            # sync so the web UI's editor/preview keeps working (plan §17).
            "instructions": entrypoint_markdown,
        },
        manifest={"name": skill.slug, "schema_version": 1},
        dependency_config={},
        change_log="Updated from desktop sync",
        version_status="draft",
        package_manifest_hash=request.package_manifest_hash,
        package_size_bytes=_package_size_bytes(session, storage, request.package_manifest_hash),
    )
    if created_version is None:
        raise AppError("SYNC_MUTATION_FAILED", "Skill update produced no version.", 500)
    receipt = _insert_receipt(
        session,
        user_id=user_id,
        device_id=device.id,
        request=request,
        result_code="acked",
        result_revision=skill.sync_revision,
        resource_id=skill.id,
        response_payload=_acked_payload(
            mutation_id, skill.id, skill.sync_revision, skill.current_package_hash
        ),
    )
    return _receipt_response(receipt)


def _handle_delete(
    session: Session,
    *,
    user_id: str,
    device: Device,
    request: SyncMutationRequest,
) -> dict[str, Any]:
    assert request.remote_skill_id is not None
    mutation_id = str(request.mutation_id)
    base_revision = request.base_revision or 0

    skill = _load_skill_locked(session, request.remote_skill_id, user_id)
    if skill is None:
        receipt = _insert_receipt(
            session,
            user_id=user_id,
            device_id=device.id,
            request=request,
            result_code="permission_denied",
            result_revision=None,
            resource_id=request.remote_skill_id,
            response_payload=_error_payload(
                mutation_id,
                "permission_denied",
                "SKILL_NOT_FOUND",
                "Skill was not found for this account.",
            ),
        )
        return _receipt_response(receipt)

    if skill.sync_revision != base_revision:
        receipt = _insert_receipt(
            session,
            user_id=user_id,
            device_id=device.id,
            request=request,
            result_code="conflict",
            result_revision=None,
            resource_id=skill.id,
            response_payload=_error_payload(
                mutation_id,
                "conflict",
                "REVISION_CONFLICT",
                "The server head has advanced past baseRevision.",
                conflict=_conflict_head(skill),
            ),
        )
        return _receipt_response(receipt)

    mutations = SkillMutationService(session, user_id)
    mutations.soft_delete(skill, audit_action="sync_skill.deleted")
    receipt = _insert_receipt(
        session,
        user_id=user_id,
        device_id=device.id,
        request=request,
        result_code="acked",
        result_revision=skill.sync_revision,
        resource_id=skill.id,
        response_payload=_acked_payload(mutation_id, skill.id, skill.sync_revision, None),
    )
    return _receipt_response(receipt)


def _package_size_bytes(session: Session, storage: BlobStorage, manifest_hash: str) -> int:
    row = session.get(SkillBlobObject, manifest_hash)
    del storage
    return int(row.size_bytes) if row is not None else 0


def apply_sync_mutation(
    session: Session,
    *,
    user_id: str,
    device: Device,
    request: SyncMutationRequest,
    storage: BlobStorage,
) -> dict[str, Any]:
    """Apply one idempotent sync mutation and return its response payload.

    Raises :class:`AppError` only for transport-grade conditions (missing
    blobs, storage backend errors) that leave no receipt behind. Definitive
    logical outcomes are recorded and returned as receipt-backed payloads.
    """
    existing_receipt = session.scalar(
        select(SyncMutationReceipt).where(
            SyncMutationReceipt.user_id == user_id,
            SyncMutationReceipt.device_id == device.id,
            SyncMutationReceipt.mutation_id == str(request.mutation_id),
        )
    )
    if existing_receipt is not None:
        # Replay: return the persisted logical result without re-applying.
        return _receipt_response(existing_receipt)

    if request.operation == OPERATION_CREATE:
        return _handle_create(
            session, user_id=user_id, device=device, request=request, storage=storage
        )
    if request.operation == OPERATION_UPDATE:
        return _handle_update(
            session, user_id=user_id, device=device, request=request, storage=storage
        )
    return _handle_delete(session, user_id=user_id, device=device, request=request)
