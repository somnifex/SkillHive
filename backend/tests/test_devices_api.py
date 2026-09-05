"""M2.3 device identity endpoint tests.

Covers idempotent registration per (user, client_instance_id), revoked-device
rejection, user isolation, and the revoke lifecycle over the HTTP surface.
"""

from collections.abc import Generator
from pathlib import Path
from typing import cast
from uuid import uuid4

import pytest
from app.db.base import Base
from app.db.session import get_db
from app.main import app
from fastapi.testclient import TestClient
from sqlalchemy import create_engine
from sqlalchemy.orm import Session, sessionmaker


@pytest.fixture
def device_session(tmp_path: Path) -> Generator[Session, None, None]:
    engine = create_engine(
        f"sqlite:///{(tmp_path / 'devices.db').as_posix()}",
        connect_args={"check_same_thread": False},
    )
    Base.metadata.create_all(engine)
    factory = sessionmaker(bind=engine, autoflush=False, expire_on_commit=False)
    with factory() as session:
        yield session
    engine.dispose()


@pytest.fixture
def device_client(
    device_session: Session, monkeypatch: pytest.MonkeyPatch
) -> Generator[TestClient, None, None]:
    def override_get_db() -> Generator[Session, None, None]:
        yield device_session

    app.dependency_overrides[get_db] = override_get_db
    with TestClient(app) as test_client:
        yield test_client
    app.dependency_overrides.clear()


def _bearer(client: TestClient, username: str) -> dict[str, str]:
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


def _register(client: TestClient, headers: dict[str, str], instance: str) -> dict[str, object]:
    response = client.post(
        "/api/v1/devices/register",
        headers=headers,
        json={
            "protocolVersion": 1,
            "clientInstanceId": instance,
            "displayName": "Howie Laptop",
            "platform": "windows",
            "appVersion": "0.2.0",
        },
    )
    assert response.status_code == 201
    return cast("dict[str, object]", response.json())


def test_register_is_idempotent_per_instance(device_client: TestClient) -> None:
    headers = _bearer(device_client, "device_owner")
    instance = str(uuid4())

    first = _register(device_client, headers, instance)
    second = _register(device_client, headers, instance)

    assert first["deviceId"] == second["deviceId"]
    assert first["clientInstanceId"] == instance
    assert first["revokedAt"] is None
    listing = device_client.get("/api/v1/devices", headers=headers)
    assert listing.status_code == 200
    assert len(listing.json()) == 1


def test_registration_updates_metadata_on_repeat(device_client: TestClient) -> None:
    headers = _bearer(device_client, "metadata_updater")
    instance = str(uuid4())
    _register(device_client, headers, instance)

    repeat = device_client.post(
        "/api/v1/devices/register",
        headers=headers,
        json={
            "protocolVersion": 1,
            "clientInstanceId": instance,
            "displayName": "Renamed Laptop",
            "platform": "windows",
            "appVersion": "0.3.0",
        },
    )
    assert repeat.status_code == 201
    assert repeat.json()["displayName"] == "Renamed Laptop"
    assert repeat.json()["appVersion"] == "0.3.0"


def test_revoked_registration_rejected(device_client: TestClient) -> None:
    headers = _bearer(device_client, "revoked_registrant")
    registered = _register(device_client, headers, str(uuid4()))

    revoke = device_client.delete(f"/api/v1/devices/{registered['deviceId']}", headers=headers)
    assert revoke.status_code == 204

    again = device_client.post(
        "/api/v1/devices/register",
        headers=headers,
        json={"protocolVersion": 1, "clientInstanceId": registered["clientInstanceId"]},
    )
    assert again.status_code == 403
    assert again.json()["error"]["code"] == "DEVICE_REVOKED"


def test_revoke_is_idempotent_and_404_for_unknown(device_client: TestClient) -> None:
    headers = _bearer(device_client, "revoker")
    registered = _register(device_client, headers, str(uuid4()))

    first = device_client.delete(f"/api/v1/devices/{registered['deviceId']}", headers=headers)
    second = device_client.delete(f"/api/v1/devices/{registered['deviceId']}", headers=headers)
    assert first.status_code == 204
    assert second.status_code == 204

    missing = device_client.delete(f"/api/v1/devices/{uuid4()}", headers=headers)
    assert missing.status_code == 404
    assert missing.json()["error"]["code"] == "DEVICE_NOT_FOUND"


def test_devices_are_isolated_per_user(device_client: TestClient) -> None:
    owner_headers = _bearer(device_client, "owner_a")
    other_headers = _bearer(device_client, "owner_b")
    registered = _register(device_client, owner_headers, str(uuid4()))

    foreign_revoke = device_client.delete(
        f"/api/v1/devices/{registered['deviceId']}", headers=other_headers
    )
    assert foreign_revoke.status_code == 404

    other_listing = device_client.get("/api/v1/devices", headers=other_headers)
    assert other_listing.json() == []
    owner_listing = device_client.get("/api/v1/devices", headers=owner_headers)
    assert len(owner_listing.json()) == 1


def test_unauthenticated_device_requests_rejected(device_client: TestClient) -> None:
    listing = device_client.get("/api/v1/devices")
    assert listing.status_code == 401
    registration = device_client.post(
        "/api/v1/devices/register",
        json={"protocolVersion": 1, "clientInstanceId": str(uuid4())},
    )
    assert registration.status_code == 401
