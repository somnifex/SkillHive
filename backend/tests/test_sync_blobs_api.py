"""M2.2 blob transport endpoint tests.

Covers missing-object negotiation and verified upload/download over the HTTP
surface, including digest/size mismatch rejection.
"""

from collections.abc import Generator
from pathlib import Path
from typing import cast

import pytest
from app.db.base import Base
from app.db.session import get_db
from app.main import app
from app.services.blob_registry import register_verified_blob
from app.services.blob_storage import (
    LocalFilesystemBlobStorage,
    hash_bytes,
    reset_blob_storage_for_tests,
)
from fastapi.testclient import TestClient
from sqlalchemy import create_engine
from sqlalchemy.orm import Session, sessionmaker


@pytest.fixture
def api_storage(tmp_path: Path) -> LocalFilesystemBlobStorage:
    return LocalFilesystemBlobStorage(tmp_path / "blobs")


@pytest.fixture
def api_session(tmp_path: Path) -> Generator[Session, None, None]:
    engine = create_engine(
        f"sqlite:///{(tmp_path / 'api.db').as_posix()}",
        connect_args={"check_same_thread": False},
    )
    Base.metadata.create_all(engine)
    factory = sessionmaker(bind=engine, autoflush=False, expire_on_commit=False)
    with factory() as session:
        yield session
    engine.dispose()


@pytest.fixture
def api_client(
    api_session: Session, api_storage: LocalFilesystemBlobStorage, monkeypatch: pytest.MonkeyPatch
) -> Generator[TestClient, None, None]:
    def override_get_db() -> Generator[Session, None, None]:
        yield api_session

    monkeypatch.setattr("app.api.v1.sync.get_blob_storage", lambda: api_storage)
    app.dependency_overrides[get_db] = override_get_db
    with TestClient(app) as test_client:
        yield test_client
    app.dependency_overrides.clear()
    reset_blob_storage_for_tests()


def _bearer(client: TestClient, username: str) -> dict[str, str]:
    response = client.post(
        "/api/v1/auth/register",
        json={
            "username": username,
            "display_name": "Blob User",
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


def test_missing_blobs_negotiation_reports_absent_objects(
    api_client: TestClient, api_session: Session, api_storage: LocalFilesystemBlobStorage
) -> None:
    headers = _bearer(api_client, "negotiator")
    payload = b"already stored"
    hash_value = hash_bytes(payload)
    register_verified_blob(api_session, api_storage, hash_value, payload)

    response = api_client.post(
        "/api/v1/sync/blobs/missing",
        headers=headers,
        json={
            "protocolVersion": 1,
            "objects": [
                {"hash": hash_value, "sizeBytes": len(payload)},
                {"hash": f"sha256:{'b' * 64}", "sizeBytes": 3},
            ],
        },
    )
    assert response.status_code == 200
    assert response.json()["missing"] == [{"hash": f"sha256:{'b' * 64}", "sizeBytes": 3}]


def test_upload_blob_round_trip(api_client: TestClient) -> None:
    headers = _bearer(api_client, "uploader")
    payload = b"package blob payload"
    hash_value = hash_bytes(payload)

    before = api_client.post(
        "/api/v1/sync/blobs/missing",
        headers=headers,
        json={"protocolVersion": 1, "objects": [{"hash": hash_value, "sizeBytes": len(payload)}]},
    )
    assert before.json()["missing"] == [{"hash": hash_value, "sizeBytes": len(payload)}]

    upload = api_client.put(
        f"/api/v1/sync/blobs/{hash_value}",
        content=payload,
        headers={**headers, "Content-Type": "application/octet-stream"},
    )
    assert upload.status_code == 204

    after = api_client.post(
        "/api/v1/sync/blobs/missing",
        headers=headers,
        json={"protocolVersion": 1, "objects": [{"hash": hash_value, "sizeBytes": len(payload)}]},
    )
    assert after.json()["missing"] == []


def test_upload_rejects_digest_mismatch(api_client: TestClient) -> None:
    headers = _bearer(api_client, "mismatch")
    declared = f"sha256:{'c' * 64}"

    response = api_client.put(
        f"/api/v1/sync/blobs/{declared}",
        content=b"these bytes hash elsewhere",
        headers={**headers, "Content-Type": "application/octet-stream"},
    )
    assert response.status_code == 400
    assert response.json()["error"]["code"] == "BLOB_DIGEST_MISMATCH"


def test_download_round_trip(
    api_client: TestClient, api_session: Session, api_storage: LocalFilesystemBlobStorage
) -> None:
    headers = _bearer(api_client, "downloader")
    payload = b"download me"
    hash_value = hash_bytes(payload)
    register_verified_blob(api_session, api_storage, hash_value, payload)

    response = api_client.get(f"/api/v1/sync/blobs/{hash_value}", headers=headers)
    assert response.status_code == 200
    assert response.content == payload
