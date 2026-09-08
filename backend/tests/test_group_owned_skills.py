from typing import Any, cast

from fastapi.testclient import TestClient
from sqlalchemy.orm import Session


def login(client: TestClient, username: str) -> dict[str, str]:
    response = client.post(
        "/api/v1/auth/login",
        json={"username": username, "password": "Strong123!"},
    )
    assert response.status_code == 200
    return {"Authorization": f"Bearer {response.json()['access_token']}"}


def register(client: TestClient, username: str) -> tuple[dict[str, str], str]:
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
    return login(client, username), str(response.json()["id"])


def add_member(
    client: TestClient,
    *,
    group_id: str,
    owner_headers: dict[str, str],
    member_headers: dict[str, str],
    username: str,
) -> None:
    invitation = client.post(
        f"/api/v1/groups/{group_id}/members/invite",
        headers=owner_headers,
        json={"identity": username},
    )
    assert invitation.status_code == 201
    accepted = client.post(
        f"/api/v1/groups/invitations/{invitation.json()['id']}/accept",
        headers=member_headers,
    )
    assert accepted.status_code == 200


def test_personal_skill_can_be_published_and_managed_in_group(
    client: TestClient,
    db_session: Session,
) -> None:
    del db_session
    owner_headers, _owner_id = register(client, "group-owner")
    author_headers, author_id = register(client, "skill-author")
    member_headers, member_id = register(client, "plain-member")

    group_response = client.post(
        "/api/v1/groups",
        headers=owner_headers,
        json={"name": "Shared Skills Team"},
    )
    assert group_response.status_code == 201
    group_id = str(group_response.json()["id"])
    add_member(
        client,
        group_id=group_id,
        owner_headers=owner_headers,
        member_headers=author_headers,
        username="skill-author",
    )
    add_member(
        client,
        group_id=group_id,
        owner_headers=owner_headers,
        member_headers=member_headers,
        username="plain-member",
    )

    personal = client.post(
        "/api/v1/skills",
        headers=author_headers,
        json={
            "name": "Incident Helper",
            "slug": "incident-helper",
            "description": "Personal draft",
            "content": {"instructions": "Triage the incident."},
        },
    )
    assert personal.status_code == 201
    personal_id = str(personal.json()["id"])

    published = client.post(
        f"/api/v1/skills/{personal_id}/publish-to-group",
        headers=author_headers,
        json={"group_id": group_id},
    )
    assert published.status_code == 201
    shared = cast(dict[str, Any], published.json())
    shared_id = str(shared["id"])
    assert shared_id != personal_id
    assert shared["skill_type"] == "group"
    assert shared["group_id"] == group_id
    assert shared["created_by"] == author_id
    assert shared["can_manage"] is True

    source = client.get(f"/api/v1/skills/{personal_id}", headers=author_headers)
    assert source.status_code == 200
    assert source.json()["skill_type"] == "private"

    visible = client.get(
        f"/api/v1/groups/{group_id}/skills/shared",
        headers=member_headers,
    )
    assert visible.status_code == 200
    assert visible.json()["total"] == 1
    assert visible.json()["items"][0]["can_manage"] is False

    denied = client.patch(
        f"/api/v1/groups/{group_id}/skills/shared/{shared_id}",
        headers=member_headers,
        json={"description": "unauthorized"},
    )
    assert denied.status_code == 403

    promoted = client.patch(
        f"/api/v1/groups/{group_id}/members/{member_id}",
        headers=owner_headers,
        json={"role": "admin"},
    )
    assert promoted.status_code == 200
    admin_update = client.patch(
        f"/api/v1/groups/{group_id}/skills/shared/{shared_id}",
        headers=member_headers,
        json={"category": "Operations"},
    )
    assert admin_update.status_code == 200

    author_update = client.patch(
        f"/api/v1/groups/{group_id}/skills/shared/{shared_id}",
        headers=author_headers,
        json={
            "description": "Maintained by the author",
            "content": {"instructions": "Triage and document the incident."},
        },
    )
    assert author_update.status_code == 200
    assert author_update.json()["description"] == "Maintained by the author"
    assert author_update.json()["current_version"]["version"] == "0.1.1"

    owner_update = client.patch(
        f"/api/v1/groups/{group_id}/skills/shared/{shared_id}",
        headers=owner_headers,
        json={"description": "Maintained by the group administrator"},
    )
    assert owner_update.status_code == 200
    assert owner_update.json()["can_manage"] is True

    versions = client.get(
        f"/api/v1/groups/{group_id}/skills/shared/{shared_id}/versions",
        headers=author_headers,
    )
    assert versions.status_code == 200
    assert len(versions.json()) == 2

    changes = client.get("/api/v1/sync/changes?limit=100", headers=member_headers)
    assert changes.status_code == 200
    assert any(
        item["resourceId"] == shared_id
        and item["metadata"].get("group_id") == group_id
        for item in changes.json()["changes"]
    )
