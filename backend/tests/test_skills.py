from typing import Any, cast

from app.models import AuditLog
from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session


def auth_header(client: TestClient, username: str) -> dict[str, str]:
    password = "Strong123!"
    register = client.post(
        "/api/v1/auth/register",
        json={
            "username": username,
            "display_name": username.title(),
            "email": f"{username}@example.com",
            "password": password,
        },
    )
    assert register.status_code == 201
    login = client.post(
        "/api/v1/auth/login",
        json={"username": username, "password": password},
    )
    assert login.status_code == 200
    return {"Authorization": f"Bearer {login.json()['access_token']}"}


def create_skill(client: TestClient, headers: dict[str, str]) -> dict[str, Any]:
    response = client.post(
        "/api/v1/skills",
        headers=headers,
        json={
            "name": "Research Assistant",
            "slug": "research-assistant",
            "description": "Summarize papers",
            "category": "Research",
            "tags": ["papers", "summary"],
            "content": {"instructions": "Summarize the supplied paper."},
        },
    )
    assert response.status_code == 201
    return cast(dict[str, Any], response.json())


def test_private_skill_crud_and_versions(client: TestClient, db_session: Session) -> None:
    headers = auth_header(client, "alice")
    created = create_skill(client, headers)
    skill_id = str(created["id"])
    assert created["current_version"]["version"] == "0.1.0"

    listing = client.get(
        "/api/v1/skills?query=Research&category=Research&tag=papers",
        headers=headers,
    )
    assert listing.status_code == 200
    assert listing.json()["total"] == 1

    updated = client.patch(
        f"/api/v1/skills/{skill_id}",
        headers=headers,
        json={
            "status": "published",
            "content": {"instructions": "Create a structured research summary."},
            "change_log": "Improve output structure",
        },
    )
    assert updated.status_code == 200
    assert updated.json()["current_version"]["version"] == "0.1.1"

    versions = client.get(f"/api/v1/skills/{skill_id}/versions", headers=headers)
    assert versions.status_code == 200
    assert {item["version"] for item in versions.json()} == {"0.1.0", "0.1.1"}

    copied = client.post(f"/api/v1/skills/{skill_id}/copy", headers=headers)
    assert copied.status_code == 201
    assert copied.json()["slug"] == "research-assistant-copy"

    deleted = client.delete(f"/api/v1/skills/{skill_id}", headers=headers)
    assert deleted.status_code == 204
    assert client.get(f"/api/v1/skills/{skill_id}", headers=headers).status_code == 404

    actions = set(db_session.scalars(select(AuditLog.action)))
    assert {
        "private_skill.created",
        "private_skill.updated",
        "private_skill.deleted",
    } <= actions


def test_user_cannot_access_another_users_private_skill(client: TestClient) -> None:
    alice_headers = auth_header(client, "alice")
    skill = create_skill(client, alice_headers)
    bob_headers = auth_header(client, "bob")
    skill_url = f"/api/v1/skills/{skill['id']}"

    assert client.get(skill_url, headers=bob_headers).status_code == 404
    assert client.patch(skill_url, headers=bob_headers, json={"name": "Stolen"}).status_code == 404
    assert client.delete(skill_url, headers=bob_headers).status_code == 404


def test_trash_restore_and_purge_lifecycle(client: TestClient) -> None:
    headers = auth_header(client, "trashy")
    created = create_skill(client, headers)
    skill_id = str(created["id"])

    # Delete lands in the trash, not in the active list.
    deleted = client.delete(f"/api/v1/skills/{skill_id}", headers=headers)
    assert deleted.status_code == 204
    active = client.get("/api/v1/skills", headers=headers)
    assert active.json()["total"] == 0

    trash = client.get("/api/v1/skills/trash", headers=headers)
    assert trash.status_code == 200
    assert trash.json()["total"] == 1
    assert trash.json()["items"][0]["id"] == skill_id

    # The slug stays reserved by the trashed original (unique constraint is
    # not status-filtered), so re-creating it is rejected until the trash
    # entry is purged.
    conflict = client.post(
        "/api/v1/skills",
        headers=headers,
        json={
            "name": "Research Assistant",
            "slug": "research-assistant",
            "description": "Summarize papers",
            "category": "Research",
            "tags": [],
            "content": {"instructions": "Summarize the supplied paper."},
        },
    )
    assert conflict.status_code == 409

    # Restore puts the original back (as draft) and both skills coexist.
    restored = client.post(f"/api/v1/skills/{skill_id}/restore", headers=headers)
    assert restored.status_code == 200
    assert restored.json()["status"] == "draft"
    assert client.get("/api/v1/skills/trash", headers=headers).json()["total"] == 0

    # Once restored, the skill is active again: restore repeats and direct
    # purge both 404 because the skill is no longer in the trash.
    assert (
        client.post(f"/api/v1/skills/{skill_id}/restore", headers=headers).status_code == 404
    )
    assert (
        client.delete(f"/api/v1/skills/{skill_id}/purge", headers=headers).status_code == 404
    )
    assert client.get("/api/v1/skills/trash", headers=headers).json()["total"] == 0


def test_restore_rejects_active_skill(client: TestClient) -> None:
    headers = auth_header(client, "restore-guard")
    created = create_skill(client, headers)
    skill_id = str(created["id"])
    # An active skill is not "in the trash", so the restore route 404s on it.
    response = client.post(f"/api/v1/skills/{skill_id}/restore", headers=headers)
    assert response.status_code == 404


def test_trash_retention_autopurge_respects_setting(
    client: TestClient, db_session: Session
) -> None:
    from datetime import timedelta

    from app.db.base import utc_now
    from app.models import Skill, SystemSetting
    from app.services.blob_storage import SETTINGS_KEY
    from app.services.trash_gc import purge_expired_trash

    headers = auth_header(client, "retention")
    created = create_skill(client, headers)
    skill_id = str(created["id"])
    assert client.delete(f"/api/v1/skills/{skill_id}", headers=headers).status_code == 204

    skill = db_session.get(Skill, skill_id)
    assert skill is not None and skill.deleted_at is not None
    skill.deleted_at = utc_now() - timedelta(days=31)
    db_session.commit()

    # Default retention (30) purges the expired entry and leaves a tombstone.
    purged = purge_expired_trash(db_session)
    assert purged == 1
    assert db_session.get(Skill, skill_id) is None

    # retention=0 disables automatic purging entirely.
    row = db_session.get(SystemSetting, SETTINGS_KEY)
    if row is None:
        row = SystemSetting(key=SETTINGS_KEY, value={})
        db_session.add(row)
    row.value = {"trash_retention_days": 0}
    db_session.commit()

    second = create_skill(client, headers)
    second_id = str(second["id"])
    assert client.delete(f"/api/v1/skills/{second_id}", headers=headers).status_code == 204
    second_row = db_session.get(Skill, second_id)
    assert second_row is not None and second_row.deleted_at is not None
    second_row.deleted_at = utc_now() - timedelta(days=400)
    db_session.commit()
    assert purge_expired_trash(db_session) == 0
    assert db_session.get(Skill, second_id) is not None


def test_version_tags_rollback_and_export(client: TestClient) -> None:
    headers = auth_header(client, "versions")
    created = create_skill(client, headers)
    skill_id = str(created["id"])

    # Create a second version.
    second = client.post(
        f"/api/v1/skills/{skill_id}/versions",
        headers=headers,
        json={
            "version": "0.2.0",
            "content": {"instructions": "Second generation instructions."},
            "change_log": "bump",
        },
    )
    assert second.status_code == 201

    # Tagging: labels are unique across the skill's versions.
    tagged = client.put(
        f"/api/v1/skills/{skill_id}/versions/0.1.0/tags",
        headers=headers,
        json={"tags": ["stable", "v1"]},
    )
    assert tagged.status_code == 200
    assert tagged.json()["tags"] == ["stable", "v1"]

    clash = client.put(
        f"/api/v1/skills/{skill_id}/versions/0.2.0/tags",
        headers=headers,
        json={"tags": ["stable"]},
    )
    assert clash.status_code == 409

    versions = client.get(f"/api/v1/skills/{skill_id}/versions", headers=headers)
    tags_by_version = {row["version"]: row["tags"] for row in versions.json()}
    assert tags_by_version["0.1.0"] == ["stable", "v1"]
    assert tags_by_version["0.2.0"] == []

    # Rollback mints a NEW version carrying the old content.
    rolled = client.post(
        f"/api/v1/skills/{skill_id}/rollback",
        headers=headers,
        json={"version": "0.1.0"},
    )
    assert rolled.status_code == 201
    body = rolled.json()
    assert body["version"] == "0.2.1"
    assert body["content"]["instructions"] == "Summarize the supplied paper."
    assert "0.1.0" in body["change_log"]
    detail = client.get(f"/api/v1/skills/{skill_id}", headers=headers)
    assert detail.json()["current_version"]["version"] == "0.2.1"

    # Version export returns a zip containing SKILL.md.
    exported = client.get(f"/api/v1/skills/{skill_id}/versions/0.1.0/export", headers=headers)
    assert exported.status_code == 200
    assert exported.headers["content-type"] == "application/zip"
    import io
    import zipfile

    with zipfile.ZipFile(io.BytesIO(exported.content)) as archive:
        names = archive.namelist()
    assert any(name.endswith("SKILL.md") for name in names)
