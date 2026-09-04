from uuid import UUID

import pytest
from pydantic import ValidationError

from app.schemas.sync import MissingBlobsRequest, SyncMutationRequest, SyncMutationResponse
from app.services.sync_cursor import SyncCursorError, decode_sync_cursor, encode_sync_cursor

_DEVICE_ID = "11111111-1111-4111-8111-111111111111"
_MUTATION_ID = "22222222-2222-4222-8222-222222222222"
_REMOTE_ID = "33333333-3333-4333-8333-333333333333"
_HASH = f"sha256:{'a' * 64}"
_METADATA = {
    "name": "Code Review",
    "slug": "code-review",
    "description": "Review code",
    "category": "Engineering",
    "tags": ["review"],
}


def test_create_mutation_accepts_protocol_v1_camel_case() -> None:
    request = SyncMutationRequest.model_validate(
        {
            "protocolVersion": 1,
            "deviceId": _DEVICE_ID,
            "mutationId": _MUTATION_ID,
            "operation": "create",
            "clientSkillId": "local-skill-1",
            "baseRevision": 0,
            "packageManifestHash": _HASH,
            "metadata": _METADATA,
        }
    )

    assert request.device_id == UUID(_DEVICE_ID)
    assert request.remote_skill_id is None
    assert request.base_revision == 0
    assert request.model_dump(by_alias=True)["packageManifestHash"] == _HASH


def test_update_requires_positive_base_revision() -> None:
    with pytest.raises(ValidationError):
        SyncMutationRequest.model_validate(
            {
                "protocolVersion": 1,
                "deviceId": _DEVICE_ID,
                "mutationId": _MUTATION_ID,
                "operation": "update",
                "clientSkillId": "local-skill-1",
                "remoteSkillId": _REMOTE_ID,
                "baseRevision": 0,
                "packageManifestHash": _HASH,
                "metadata": _METADATA,
            }
        )


def test_delete_rejects_package_payload() -> None:
    with pytest.raises(ValidationError):
        SyncMutationRequest.model_validate(
            {
                "protocolVersion": 1,
                "deviceId": _DEVICE_ID,
                "mutationId": _MUTATION_ID,
                "operation": "delete",
                "clientSkillId": "local-skill-1",
                "remoteSkillId": _REMOTE_ID,
                "baseRevision": 3,
                "packageManifestHash": _HASH,
            }
        )


def test_protocol_version_is_rejected_before_endpoint_logic() -> None:
    with pytest.raises(ValidationError):
        SyncMutationRequest.model_validate(
            {
                "protocolVersion": 2,
                "deviceId": _DEVICE_ID,
                "mutationId": _MUTATION_ID,
                "operation": "delete",
                "clientSkillId": "local-skill-1",
                "remoteSkillId": _REMOTE_ID,
                "baseRevision": 3,
            }
        )


def test_blob_negotiation_rejects_duplicate_hashes() -> None:
    with pytest.raises(ValidationError):
        MissingBlobsRequest.model_validate(
            {
                "protocolVersion": 1,
                "objects": [
                    {"hash": _HASH, "sizeBytes": 10},
                    {"hash": _HASH, "sizeBytes": 10},
                ],
            }
        )


def test_mutation_response_shape_is_status_specific() -> None:
    acknowledged = SyncMutationResponse.model_validate(
        {
            "protocolVersion": 1,
            "mutationId": _MUTATION_ID,
            "status": "acked",
            "result": {
                "remoteSkillId": _REMOTE_ID,
                "revision": 1,
                "packageManifestHash": _HASH,
            },
        }
    )
    assert acknowledged.result is not None

    with pytest.raises(ValidationError):
        SyncMutationResponse.model_validate(
            {
                "protocolVersion": 1,
                "mutationId": _MUTATION_ID,
                "status": "acked",
            }
        )

    with pytest.raises(ValidationError):
        SyncMutationResponse.model_validate(
            {
                "protocolVersion": 1,
                "mutationId": _MUTATION_ID,
                "status": "conflict",
                "result": {"remoteSkillId": _REMOTE_ID, "revision": 2},
            }
        )


def test_cursor_round_trip_is_opaque_and_stable() -> None:
    for sequence in (0, 1, 42, 2**64 - 1):
        cursor = encode_sync_cursor(sequence)
        assert cursor.startswith("v1.")
        assert decode_sync_cursor(cursor) == sequence

    assert decode_sync_cursor(None) == 0
    assert decode_sync_cursor("") == 0


@pytest.mark.parametrize("cursor", ["v2.AAAAAAAA", "v1.", "v1.!", "not-a-cursor"])
def test_cursor_rejects_malformed_or_unknown_versions(cursor: str) -> None:
    with pytest.raises(SyncCursorError):
        decode_sync_cursor(cursor)
