"""M2.2 blob transport endpoints.

Missing-object negotiation and verified bounded upload. Endpoints are
authenticated with the standard bearer-token dependency; device-scoped
authorization is tightened in M2.3 when device identity lands.
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
)
from app.services.blob_registry import missing_blobs, register_verified_blob
from app.services.blob_storage import get_blob_storage

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
    storage = get_blob_storage()
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
    storage = get_blob_storage()

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
) -> StreamingResponse:
    """Stream a verified stored object back to the client."""
    del user  # Package ownership is enforced at mutation level; blob bytes
    # are content-addressed and carry no authorization boundary of their own.
    storage = get_blob_storage()
    stream = storage.open(hash_value)
    return StreamingResponse(stream, media_type="application/octet-stream")
