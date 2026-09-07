"""M2.2/M2.4 sync transport endpoints.

Missing-object negotiation, verified bounded upload, and the idempotent
mutation push endpoint. Endpoints are authenticated with the standard
bearer-token dependency.
"""

import hashlib
from typing import Annotated

from fastapi import APIRouter, Depends, Request
from fastapi.responses import StreamingResponse
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.db.session import get_db
from app.permissions.dependencies import CurrentUser
from app.schemas.sync import (
    MAX_BLOB_BYTES,
    MissingBlobsRequest,
    MissingBlobsResponse,
    SyncChangesResponse,
    SyncMutationRequest,
    SyncMutationResponse,
)
from app.services.blob_registry import missing_blobs, register_verified_blob
from app.services.blob_storage import get_blob_storage
from app.services.sync_changes import list_changes
from app.services.sync_mutations import apply_sync_mutation, validate_active_device

router = APIRouter(prefix="/sync", tags=["sync"])

DBSession = Annotated[Session, Depends(get_db)]


@router.post("/blobs/missing", response_model=MissingBlobsResponse)
def negotiate_missing_blobs(
    request: MissingBlobsRequest,
    user: CurrentUser,
    session: DBSession,
) -> MissingBlobsResponse:
    """Report which of the declared objects the server still needs."""
    del user  # Object ownership is enforced at mutation commit, not at blob
    # presence checks; any authenticated user may negotiate.
    storage = get_blob_storage(session)
    missing = missing_blobs(session, storage, request.objects)
    return MissingBlobsResponse(missing=missing)


@router.put("/blobs/{hash_value}", status_code=204)
async def upload_blob(
    hash_value: str,
    request: Request,
    user: CurrentUser,
    session: DBSession,
) -> None:
    """Verified streaming upload of one missing object.

    The digest is recomputed while the stream is consumed; a mismatch or an
    oversized body is rejected before the object becomes addressable.
    """
    del user
    storage = get_blob_storage(session)

    declared_size: int | None = None
    if content_length := request.headers.get("content-length"):
        try:
            declared_size = int(content_length)
        except ValueError as exc:
            raise AppError("BLOB_INVALID_SIZE", "Content-Length is not an integer.", 400) from exc
        if declared_size > MAX_BLOB_BYTES:
            raise AppError("BLOB_TOO_LARGE", "Blob exceeds the per-object size limit.", 413)

    digest = hashlib.sha256()
    payload = bytearray()
    async for chunk in request.stream():
        payload.extend(chunk)
        digest.update(chunk)
        if len(payload) > MAX_BLOB_BYTES:
            raise AppError("BLOB_TOO_LARGE", "Blob exceeds the per-object size limit.", 413)

    if declared_size is not None and len(payload) != declared_size:
        raise AppError(
            "BLOB_SIZE_MISMATCH",
            "Stream length does not match the declared Content-Length.",
            400,
        )

    canonical = f"sha256:{hash_value.removeprefix('sha256:')}"
    if canonical != f"sha256:{digest.hexdigest()}":
        raise AppError(
            "BLOB_DIGEST_MISMATCH",
            "Uploaded blob digest does not match its content address.",
            400,
        )

    register_verified_blob(session, storage, canonical, bytes(payload))
    session.commit()


@router.get("/blobs/{hash_value}")
def download_blob(
    hash_value: str,
    user: CurrentUser,
    session: DBSession,
) -> StreamingResponse:
    """Stream a verified stored object back to the client."""
    del user  # Package ownership is enforced at mutation level; blob bytes
    # are content-addressed and carry no authorization boundary of their own.
    storage = get_blob_storage(session)
    stream = storage.open(hash_value)
    return StreamingResponse(stream, media_type="application/octet-stream")

@router.post("/mutations", response_model=SyncMutationResponse)
def submit_mutation(
    request: SyncMutationRequest,
    user: CurrentUser,
    session: DBSession,
) -> SyncMutationResponse:
    """Apply one idempotent desktop mutation (M2.4).

    The whole apply — receipt lookup, domain mutation, change event, audit,
    receipt — is one transaction; a committed receipt is replayed verbatim on
    retry. Definitive logical outcomes (conflict/permission/validation) are
    returned as protocol responses with HTTP 200; only transport-grade
    conditions (missing blobs, revoked device) surface as HTTP errors.
    """
    device = validate_active_device(session, user.id, str(request.device_id))
    payload = apply_sync_mutation(
        session,
        user_id=user.id,
        device=device,
        request=request,
        storage=get_blob_storage(session),
    )
    session.commit()
    return SyncMutationResponse.model_validate(payload)


@router.get("/changes", response_model=SyncChangesResponse)
def pull_changes(
    user: CurrentUser,
    session: DBSession,
    cursor: str | None = None,
    limit: int = 100,
) -> SyncChangesResponse:
    """Incremental pull over the durable change feed (M2.5).

    Pages are ordered by the append-only change-log sequence; the returned
    cursor encodes that sequence so a resumed pull never re-reads committed
    pages. Visibility follows current server authorization: private Skills
    are owner-only, published global Skills are visible, and delete events
    are explicit tombstones.
    """
    return list_changes(session, user_id=user.id, cursor=cursor, limit=limit)
