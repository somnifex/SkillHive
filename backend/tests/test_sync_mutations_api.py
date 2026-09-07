"""M2.4 idempotent mutation push tests.

Covers the receipt-keyed idempotency transaction pattern over the HTTP
surface: replay safety, base-revision conflicts, package closure
verification, per-user authorization, and revoked-device rejection.
"""

from collections.abc import Generator
from pathlib import Path
from typing import Any, cast
from uuid import uuid4

import pytest
from app.db.base import Base
from app.db.session import get_db
from app.main import app
from app.services.blob_registry import register_verified_blob
from app.services.blob_storage import (
    LocalFilesystemBlobStorage,
    reset_blob_storage_for_tests,
)
from fastapi.testclient import TestClient
from sqlalchemy import create_engine
from sqlalchemy.orm import Session, sessionmaker


@pytest.fixture
def push_session(tmp_path: Path) -> Generator[Session, None, None]:
    engine = create_engine(
        f"sqlite:///{(tmp_path / 'push.db').as_posix()}",
        connect_args={"check_same_thread": False},
    )
    Base.metadata.create_all(engine)
    factory = sessionmaker(bind=engine, autoflush=False, expire_on_commit=False)
    with factory() as session:
        yield session
    engine.dispose()


@pytest.fixture
def push_storage(tmp_path: Path) -> LocalFilesystemBlobStorage:
    return LocalFilesystemBlobStorage(tmp_path / "blobs")


@pytest.fixture
def push_client(
    push_session: Session,
    push_storage: LocalFilesystemBlobStorage,
    monkeypatch: pytest.MonkeyPatch,
) -> Generator[TestClient, None, None]:
    def override_get_db() -> Generator[Session, None, None]:
        yield push_session

    monkeypatch.setattr("app.api.v1.sync.get_blob_storage", lambda _session=None: push_storage)
    app.dependency_overrides[get_db] = override_get_db
    with TestClient(app) as test_client:
        yield test_client
    app.dependency_overrides.clear()
    reset_blob_storage_for_tests()


def _register_user(client: TestClient, username: str) -> dict[str, str]:
    response = client.post(
        "/api/v1/auth/register",
        json={
            "username": username,
            "display_name": f"{username} Display",
            "email": f"{username}@example.com",
            "password": "Strong123!",
        },
    )
    assert response.status_code == 201
    login = client.post(
        "/api/v1/auth/login",
        json={"username": username, "password": "Strong123!"},
    )
    assert login.status_code == 200
    token = cast("str", login.json()["access_token"])
    return {"Authorization": f"Bearer {token}"}


def _register_device(client: TestClient, headers: dict[str, str]) -> dict[str, Any]:
    response = client.post(
        "/api/v1/devices/register",
        headers=headers,
        json={
            "protocolVersion": 1,
            "clientInstanceId": str(uuid4()),
            "displayName": "Sync Desktop",
            "platform": "windows",
            "appVersion": "0.2.0",
        },
    )
    assert response.status_code == 201
    return cast("dict[str, Any]", response.json())


def _upload_package(
    client: TestClient,
    headers: dict[str, str],
    storage: LocalFilesystemBlobStorage,
    session: Session,
    skill_md: str,
) -> str:
    """Upload one SKILL.md blob plus its manifest and return the manifest hash."""
    del client, headers  # blobs are registered directly through the registry
    import hashlib
    import json

    file_hash = "sha256:" + hashlib.sha256(skill_md.encode("utf-8")).hexdigest()
    register_verified_blob(session, storage, file_hash, skill_md.encode("utf-8"))
    manifest = {
        "format_version": 1,
        "files": [
            {"path": "SKILL.md", "blobHash": file_hash, "sizeBytes": len(skill_md.encode("utf-8"))}
        ],
    }
    manifest_bytes = json.dumps(manifest).encode("utf-8")
    manifest_hash = "sha256:" + hashlib.sha256(manifest_bytes).hexdigest()
    register_verified_blob(session, storage, manifest_hash, manifest_bytes)
    return manifest_hash


def _mutation_request(
    operation: str,
    *,
    device_id: str,
    mutation_id: str | None = None,
    client_skill_id: str = "local-1",
    remote_skill_id: str | None = None,
    base_revision: int | None = None,
    package_manifest_hash: str | None = None,
    metadata: dict[str, Any] | None = None,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "protocolVersion": 1,
        "deviceId": device_id,
        "mutationId": mutation_id or str(uuid4()),
        "operation": operation,
        "clientSkillId": client_skill_id,
    }
    if remote_skill_id is not None:
        payload["remoteSkillId"] = remote_skill_id
    if base_revision is not None:
        payload["baseRevision"] = base_revision
    if package_manifest_hash is not None:
        payload["packageManifestHash"] = package_manifest_hash
    if metadata is not None:
        payload["metadata"] = metadata
    return payload


_METADATA = {
    "name": "Desktop Skill",
    "slug": "desktop-skill",
    "description": "Created offline",
    "category": "general",
    "tags": ["desktop"],
}


def test_create_mutation_acks_and_replays_receipt(
    push_client: TestClient,
    push_session: Session,
    push_storage: LocalFilesystemBlobStorage,
) -> None:
    headers = _register_user(push_client, "creator")
    device = _register_device(push_client, headers)
    manifest_hash = _upload_package(
        push_client, headers, push_storage, push_session, "# Demo skill"
    )
    request = _mutation_request(
        "create",
        device_id=device["deviceId"],
        package_manifest_hash=manifest_hash,
        metadata=_METADATA,
    )

    first = push_client.post("/api/v1/sync/mutations", headers=headers, json=request)
    assert first.status_code == 200
    body = first.json()
    assert body["status"] == "acked"
    assert body["result"]["revision"] == 1
    assert body["result"]["packageManifestHash"] == manifest_hash
    remote_skill_id = body["result"]["remoteSkillId"]

    replay = push_client.post("/api/v1/sync/mutations", headers=headers, json=request)
    assert replay.status_code == 200
    assert replay.json() == body

    # Exactly one server Skill exists: replay did not re-apply the mutation.
    listing = push_client.get("/api/v1/skills", headers=headers)
    items = [s for s in listing.json()["items"] if s["slug"] == "desktop-skill"]
    assert len(items) == 1
    assert items[0]["id"] == remote_skill_id

    # Legacy synthesis: the web UI's skill_markdown mirrors the SKILL.md
    # entrypoint of the uploaded package (plan §17), and the legacy
    # `instructions` body is mirrored so the web editor keeps working.
    detail = push_client.get(f"/api/v1/skills/{remote_skill_id}", headers=headers)
    assert detail.status_code == 200
    content = detail.json()["current_version"]["content"]
    assert content["skill_markdown"] == "# Demo skill"
    assert content["instructions"] == "# Demo skill"


def test_update_mutation_enforces_base_revision(
    push_client: TestClient,
    push_session: Session,
    push_storage: LocalFilesystemBlobStorage,
) -> None:
    headers = _register_user(push_client, "editor")
    device = _register_device(push_client, headers)
    manifest_a = _upload_package(push_client, headers, push_storage, push_session, "# A")
    created = push_client.post(
        "/api/v1/sync/mutations",
        headers=headers,
        json=_mutation_request(
            "create",
            device_id=device["deviceId"],
            package_manifest_hash=manifest_a,
            metadata=_METADATA,
        ),
    )
    remote_skill_id = created.json()["result"]["remoteSkillId"]
    revision = created.json()["result"]["revision"]

    manifest_b = _upload_package(push_client, headers, push_storage, push_session, "# B")
    stale = push_client.post(
        "/api/v1/sync/mutations",
        headers=headers,
        json=_mutation_request(
            "update",
            device_id=device["deviceId"],
            remote_skill_id=remote_skill_id,
            base_revision=revision + 5,
            package_manifest_hash=manifest_b,
            metadata={**_METADATA, "name": "Renamed"},
        ),
    )
    assert stale.status_code == 200
    stale_body = stale.json()
    assert stale_body["status"] == "conflict"
    assert stale_body["conflict"]["remoteSkillId"] == remote_skill_id
    assert stale_body["conflict"]["revision"] == revision

    current = push_client.post(
        "/api/v1/sync/mutations",
        headers=headers,
        json=_mutation_request(
            "update",
            device_id=device["deviceId"],
            remote_skill_id=remote_skill_id,
            base_revision=revision,
            package_manifest_hash=manifest_b,
            metadata={**_METADATA, "name": "Renamed"},
        ),
    )
    assert current.status_code == 200
    assert current.json()["status"] == "acked"
    assert current.json()["result"]["revision"] == revision + 1


def test_missing_blob_upload_leaves_mutation_retryable(
    push_client: TestClient,
    push_session: Session,
    push_storage: LocalFilesystemBlobStorage,
) -> None:
    headers = _register_user(push_client, "retryer")
    device = _register_device(push_client, headers)
    missing_hash = f"sha256:{'d' * 64}"

    response = push_client.post(
        "/api/v1/sync/mutations",
        headers=headers,
        json=_mutation_request(
            "create",
            device_id=device["deviceId"],
            package_manifest_hash=missing_hash,
            metadata=_METADATA,
        ),
    )
    assert response.status_code == 409
    assert response.json()["error"]["code"] == "BLOB_MISSING"

    from app.models import SyncMutationReceipt

    count = len(push_session.query(SyncMutationReceipt).all())
    assert count == 0


def test_delete_mutation_tombstones_with_revision(
    push_client: TestClient,
    push_session: Session,
    push_storage: LocalFilesystemBlobStorage,
) -> None:
    headers = _register_user(push_client, "deleter")
    device = _register_device(push_client, headers)
    manifest_hash = _upload_package(push_client, headers, push_storage, push_session, "# Bye")
    created = push_client.post(
        "/api/v1/sync/mutations",
        headers=headers,
        json=_mutation_request(
            "create",
            device_id=device["deviceId"],
            package_manifest_hash=manifest_hash,
            metadata=_METADATA,
        ),
    )
    remote_skill_id = created.json()["result"]["remoteSkillId"]
    revision = created.json()["result"]["revision"]

    deleted = push_client.post(
        "/api/v1/sync/mutations",
        headers=headers,
        json=_mutation_request(
            "delete",
            device_id=device["deviceId"],
            remote_skill_id=remote_skill_id,
            base_revision=revision,
        ),
    )
    assert deleted.status_code == 200
    assert deleted.json()["status"] == "acked"
    assert deleted.json()["result"]["revision"] == revision + 1

    replay = push_client.post(
        "/api/v1/sync/mutations",
        headers=headers,
        json=_mutation_request(
            "delete",
            device_id=device["deviceId"],
            mutation_id=deleted.json()["mutationId"],
            remote_skill_id=remote_skill_id,
            base_revision=revision,
        ),
    )
    assert replay.json() == deleted.json()


def test_foreign_device_and_revoked_device_rejected(
    push_client: TestClient,
    push_session: Session,
    push_storage: LocalFilesystemBlobStorage,
) -> None:
    owner = _register_user(push_client, "device_owner")
    device = _register_device(push_client, owner)

    response = push_client.post(
        "/api/v1/sync/mutations",
        headers=owner,
        json=_mutation_request(
            "create",
            device_id=str(uuid4()),
            package_manifest_hash=f"sha256:{'e' * 64}",
            metadata=_METADATA,
        ),
    )
    assert response.status_code == 404
    assert response.json()["error"]["code"] == "DEVICE_NOT_FOUND"

    device = _register_device(push_client, owner)
    revoke = push_client.delete(f"/api/v1/devices/{device['deviceId']}", headers=owner)
    assert revoke.status_code == 204

    blocked = push_client.post(
        "/api/v1/sync/mutations",
        headers=owner,
        json=_mutation_request(
            "create",
            device_id=device["deviceId"],
            package_manifest_hash=f"sha256:{'f' * 64}",
            metadata=_METADATA,
        ),
    )
    assert blocked.status_code == 403
    assert blocked.json()["error"]["code"] == "DEVICE_REVOKED"
