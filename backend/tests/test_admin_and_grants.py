from typing import Any, cast

from app.core.security import hash_password
from app.models import User
from fastapi.testclient import TestClient
from sqlalchemy.orm import Session


def login(client: TestClient, username: str, password: str) -> dict[str, str]:
    response = client.post(
        "/api/v1/auth/login",
        json={"username": username, "password": password},
    )
    assert response.status_code == 200
    return {"Authorization": f"Bearer {response.json()['access_token']}"}


def registered_user(client: TestClient, username: str) -> tuple[dict[str, str], str]:
    response = client.post(
        "/api/v1/auth/register",
        json={
            "username": username,
            "display_name": username.title(),
            "email": f"{username}@example.com",
            "password": "Strong123!",
        },
    )
    assert response.status_code == 201
    return login(client, username, "Strong123!"), str(response.json()["id"])


def test_global_skill_publish_group_grant_and_disable(
    client: TestClient,
    db_session: Session,
) -> None:
    admin = User(
        username="admin",
        display_name="Administrator",
        email="admin@example.com",
        password_hash=hash_password("Admin123!"),
        status="active",
        is_global_admin=True,
    )
    db_session.add(admin)
    db_session.commit()
    admin_headers = login(client, "admin", "Admin123!")
    owner_headers, _ = registered_user(client, "owner")
    member_headers, member_id = registered_user(client, "member")

    denied = client.get("/api/v1/admin/users", headers=owner_headers)
    assert denied.status_code == 403
    assert denied.json()["error"]["code"] == "PERMISSION_DENIED"

    group_response = client.post(
        "/api/v1/groups",
        headers=owner_headers,
        json={"name": "Product Team"},
    )
    group_id = str(group_response.json()["id"])
    invitation = client.post(
        f"/api/v1/groups/{group_id}/members/invite",
        headers=owner_headers,
        json={"identity": "member"},
    )
    client.post(
        f"/api/v1/groups/invitations/{invitation.json()['id']}/accept",
        headers=member_headers,
    )

    created_response = client.post(
        "/api/v1/admin/skills",
        headers=admin_headers,
        json={
            "name": "Release Notes",
            "slug": "release-notes",
            "description": "Generate concise release notes",
            "category": "Engineering",
            "tags": ["release"],
            "content": {"instructions": "Summarize user-visible changes."},
            "version": "1.0.0",
        },
    )
    assert created_response.status_code == 201
    created = cast(dict[str, Any], created_response.json())
    skill_id = str(created["id"])
    version_id = str(created["current_version"]["id"])

    published = client.post(
        f"/api/v1/admin/skills/{skill_id}/publish",
        headers=admin_headers,
        json={"version_id": version_id},
    )
    assert published.status_code == 200
    assert published.json()["status"] == "published"

    catalog = client.get(
        f"/api/v1/groups/{group_id}/skills/catalog",
        headers=owner_headers,
    )
    assert catalog.status_code == 200
    assert catalog.json()[0]["id"] == skill_id
    catalog_versions = client.get(
        f"/api/v1/groups/{group_id}/skills/catalog/{skill_id}/versions",
        headers=owner_headers,
    )
    assert catalog_versions.status_code == 200
    assert catalog_versions.json()[0]["version"] == "1.0.0"

    member_denied = client.post(
        f"/api/v1/groups/{group_id}/skills/{skill_id}",
        headers=member_headers,
        json={"version_policy": "latest"},
    )
    assert member_denied.status_code == 403

    granted = client.post(
        f"/api/v1/groups/{group_id}/skills/{skill_id}",
        headers=owner_headers,
        json={"version_policy": "locked", "locked_version_id": version_id},
    )
    assert granted.status_code == 201
    assert granted.json()["version_policy"] == "locked"
    assert granted.json()["effective_version"]["version"] == "1.0.0"

    visible = client.get(
        f"/api/v1/groups/{group_id}/skills",
        headers=member_headers,
    )
    assert visible.status_code == 200
    assert len(visible.json()) == 1

    grants = client.get(
        f"/api/v1/admin/skills/{skill_id}/grants",
        headers=admin_headers,
    )
    assert grants.status_code == 200
    assert grants.json()[0]["group_id"] == group_id

    audits = client.get(
        "/api/v1/admin/audit-logs?resource_type=skill",
        headers=admin_headers,
    )
    assert audits.status_code == 200
    assert audits.json()["total"] >= 1

    disabled = client.post(
        f"/api/v1/admin/skills/{skill_id}/disable",
        headers=admin_headers,
    )
    assert disabled.status_code == 200
    assert (
        client.get(
            f"/api/v1/groups/{group_id}/skills",
            headers=member_headers,
        ).json()
        == []
    )

    user_disabled = client.patch(
        f"/api/v1/admin/users/{member_id}/status",
        headers=admin_headers,
        json={"status": "disabled"},
    )
    assert user_disabled.status_code == 200
    assert (
        client.post(
            "/api/v1/auth/login",
            json={"username": "member", "password": "Strong123!"},
        ).status_code
        == 403
    )
