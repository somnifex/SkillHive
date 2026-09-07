import io
from pathlib import Path

from app.core.security import hash_password
from app.models import User
from app.services.blob_storage import (
    LocalFilesystemBlobStorage,
    RoutedBlobStorage,
    S3BlobStorage,
    hash_bytes,
)
from fastapi.testclient import TestClient
from pytest import MonkeyPatch
from sqlalchemy.orm import Session


def global_admin_auth(client: TestClient, db_session: Session) -> dict[str, str]:
    admin = User(
        username="rootadmin",
        display_name="Root Administrator",
        email="rootadmin@example.com",
        password_hash=hash_password("Admin123!"),
        status="active",
        is_global_admin=True,
    )
    db_session.add(admin)
    db_session.commit()
    login = client.post(
        "/api/v1/auth/login",
        json={"username": "rootadmin", "password": "Admin123!"},
    )
    assert login.status_code == 200
    return {"Authorization": f"Bearer {login.json()['access_token']}"}


def user_auth(client: TestClient, username: str) -> tuple[dict[str, str], str]:
    password = "Strong123!"
    registered = client.post(
        "/api/v1/auth/register",
        json={
            "username": username,
            "display_name": username.title(),
            "email": f"{username}@example.com",
            "password": password,
        },
    )
    assert registered.status_code == 201
    user_id = str(registered.json()["id"])
    login = client.post(
        "/api/v1/auth/login",
        json={"username": username, "password": password},
    )
    assert login.status_code == 200
    return {"Authorization": f"Bearer {login.json()['access_token']}"}, user_id


class FakeS3Error(Exception):
    def __init__(self, code: str, status: int) -> None:
        super().__init__(code)
        self.response: dict[str, object] = {
            "Error": {"Code": code},
            "ResponseMetadata": {"HTTPStatusCode": status},
        }


class FakeS3Client:
    """Minimal in-memory S3 client speaking the subset the backend uses."""

    def __init__(self) -> None:
        self.objects: dict[str, bytes] = {}

    def head_object(self, *, Bucket: str, Key: str) -> dict[str, int]:
        if Key not in self.objects:
            raise FakeS3Error("404", 404)
        return {"ContentLength": len(self.objects[Key])}

    def get_object(self, *, Bucket: str, Key: str) -> dict[str, io.BytesIO]:
        if Key not in self.objects:
            raise FakeS3Error("404", 404)
        return {"Body": io.BytesIO(self.objects[Key])}

    def put_object(self, *, Bucket: str, Key: str, Body: bytes) -> dict[str, int]:
        self.objects[Key] = bytes(Body)
        return {}

    def delete_object(self, *, Bucket: str, Key: str) -> dict[str, int]:
        self.objects.pop(Key, None)
        return {}


def _error_code(exc: Exception) -> str | None:
    return getattr(exc, "code", None)


def test_s3_storage_contract() -> None:
    client = FakeS3Client()
    storage = S3BlobStorage(client, bucket="skillhive", prefix="prod")
    payload = b"package-bytes"
    hash_value = hash_bytes(payload)

    storage.put_verified(hash_value, payload)
    digest = hash_value.split(":")[1]
    assert list(client.objects) == [f"prod/sha256/{digest[:2]}/{digest}"]
    assert storage.exists(hash_value, len(payload))
    assert not storage.exists(hash_value, len(payload) + 1)
    assert storage.open(hash_value).read() == payload

    try:
        storage.put_verified(hash_bytes(b"different-body"), payload)
    except Exception as exc:  # noqa: BLE001
        assert getattr(exc, "code", None) == "BLOB_DIGEST_MISMATCH"
    else:  # pragma: no cover
        raise AssertionError("digest mismatch not rejected")

    missing = hash_bytes(b"never-stored")
    try:
        storage.open(missing)
    except Exception as exc:  # noqa: BLE001
        assert getattr(exc, "code", None) == "BLOB_NOT_FOUND"
    else:  # pragma: no cover
        raise AssertionError("missing object not reported")

    storage.delete(hash_value)
    assert not storage.exists(hash_value, len(payload))


def test_routed_storage_falls_back_across_backends(tmp_path: Path) -> None:
    local = LocalFilesystemBlobStorage(tmp_path / "local")
    payload = b"routed-blob"
    hash_value = hash_bytes(payload)
    local.put_verified(hash_value, payload)

    routed = RoutedBlobStorage(
        default=S3BlobStorage(FakeS3Client(), "bucket", ""),
        fallbacks=[local],
    )
    assert routed.backend_name == "s3"
    # The object lives on the filesystem backend; the routed read finds it.
    assert routed.exists(hash_value, len(payload))
    assert routed.open(hash_value).read() == payload

    # New writes land on the default (S3) backend.
    fresh_payload = b"fresh-object"
    fresh = hash_bytes(fresh_payload)
    routed.put_verified(fresh, fresh_payload)
    assert routed.exists(fresh, len(fresh_payload))
    assert routed.open(fresh).read() == fresh_payload


def test_system_settings_api_and_registration_toggle(
    client: TestClient,
    db_session: Session,
    monkeypatch: MonkeyPatch,
) -> None:
    owner_headers, _ = user_auth(client, "owner")
    denied = client.get("/api/v1/admin/system/settings", headers=owner_headers)
    assert denied.status_code == 403

    monkeypatch.delenv("S3_ACCESS_KEY_ID", raising=False)
    monkeypatch.delenv("S3_SECRET_ACCESS_KEY", raising=False)

    admin_headers = global_admin_auth(client, db_session)
    initial = client.get("/api/v1/admin/system/settings", headers=admin_headers)
    assert initial.status_code == 200
    assert initial.json()["blob_storage_backend"] == "local"
    assert initial.json()["allow_registration"] is True
    assert initial.json()["s3_credentials_configured"] is False

    # Switching to S3 without credentials in the environment is refused.
    refused = client.patch(
        "/api/v1/admin/system/settings",
        headers=admin_headers,
        json={"blob_storage_backend": "s3", "s3_bucket": "skillhive"},
    )
    assert refused.status_code == 409

    monkeypatch.setenv("S3_ACCESS_KEY_ID", "test-key")
    monkeypatch.setenv("S3_SECRET_ACCESS_KEY", "test-secret")
    ready = client.patch(
        "/api/v1/admin/system/settings",
        headers=admin_headers,
        json={"blob_storage_backend": "s3", "s3_bucket": "skillhive", "s3_prefix": "prod"},
    )
    assert ready.status_code == 200
    assert ready.json()["blob_storage_backend"] == "s3"
    assert ready.json()["s3_credentials_configured"] is True

    disabled = client.patch(
        "/api/v1/admin/system/settings",
        headers=admin_headers,
        json={"allow_registration": False},
    )
    assert disabled.status_code == 200
    assert disabled.json()["allow_registration"] is False

    blocked = client.post(
        "/api/v1/auth/register",
        json={
            "username": "newcomer",
            "display_name": "Newcomer",
            "email": "newcomer@example.com",
            "password": "Strong123!",
        },
    )
    assert blocked.status_code == 403
    assert blocked.json()["error"]["code"] == "REGISTRATION_DISABLED"

    # An explicit null resets the entry to the default (registration open).
    reset = client.patch(
        "/api/v1/admin/system/settings",
        headers=admin_headers,
        json={"allow_registration": None},
    )
    assert reset.status_code == 200
    assert reset.json()["allow_registration"] is True


def test_admin_user_lifecycle(client: TestClient, db_session: Session) -> None:
    admin_headers = global_admin_auth(client, db_session)

    created = client.post(
        "/api/v1/admin/users",
        headers=admin_headers,
        json={
            "username": "provisioned",
            "display_name": "Provisioned User",
            "email": "provisioned@example.com",
            "password": "Strong123!",
        },
    )
    assert created.status_code == 201
    user_id = created.json()["id"]
    login = client.post(
        "/api/v1/auth/login",
        json={"username": "provisioned", "password": "Strong123!"},
    )
    assert login.status_code == 200

    duplicate = client.post(
        "/api/v1/admin/users",
        headers=admin_headers,
        json={
            "username": "provisioned",
            "display_name": "Dup",
            "email": "other@example.com",
            "password": "Strong123!",
        },
    )
    assert duplicate.status_code == 409

    reset = client.post(
        f"/api/v1/admin/users/{user_id}/reset-password",
        headers=admin_headers,
        json={"new_password": "Rotated456!"},
    )
    assert reset.status_code == 204
    assert (
        client.post(
            "/api/v1/auth/login",
            json={"username": "provisioned", "password": "Strong123!"},
        ).status_code
        == 401
    )
    rotated = client.post(
        "/api/v1/auth/login",
        json={"username": "provisioned", "password": "Rotated456!"},
    )
    assert rotated.status_code == 200

    # A user who still owns resources cannot be deleted.
    user_headers = {"Authorization": f"Bearer {rotated.json()['access_token']}"}
    group = client.post("/api/v1/groups", headers=user_headers, json={"name": "Owned"})
    assert group.status_code == 201
    group_id = group.json()["id"]
    blocked = client.delete(f"/api/v1/admin/users/{user_id}", headers=admin_headers)
    assert blocked.status_code == 409
    assert blocked.json()["error"]["code"] == "USER_OWNS_RESOURCES"
    assert client.delete(f"/api/v1/groups/{group_id}", headers=admin_headers).status_code == 204

    assert client.delete(f"/api/v1/admin/users/{user_id}", headers=admin_headers).status_code == 204
    assert (
        client.post(
            "/api/v1/auth/login",
            json={"username": "provisioned", "password": "Rotated456!"},
        ).status_code
        == 403
    )

    # The admin cannot delete their own account.
    me = client.get("/api/v1/auth/me", headers=admin_headers)
    admin_id = str(me.json()["id"])
    self_delete = client.delete(
        f"/api/v1/admin/users/{admin_id}", headers=admin_headers
    )
    assert self_delete.status_code == 409


def test_admin_group_tree_endpoint(client: TestClient, db_session: Session) -> None:
    owner_headers, _ = user_auth(client, "owner")
    root = client.post("/api/v1/groups", headers=owner_headers, json={"name": "Root"})
    root_id = str(root.json()["id"])
    sub = client.post(
        "/api/v1/groups",
        headers=owner_headers,
        json={"name": "Sub", "parent_group_id": root_id},
    )
    sub_id = str(sub.json()["id"])

    user_headers, _ = user_auth(client, "outsider")
    denied = client.get("/api/v1/admin/groups/tree", headers=user_headers)
    assert denied.status_code == 403

    admin_headers = global_admin_auth(client, db_session)
    tree = client.get("/api/v1/admin/groups/tree", headers=admin_headers)
    assert tree.status_code == 200
    assert {str(item["id"]) for item in tree.json()} == {root_id, sub_id}