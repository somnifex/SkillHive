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
