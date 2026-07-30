from app.core.security import hash_password
from app.models import User
from fastapi.testclient import TestClient
from sqlalchemy.orm import Session


def authenticate(client: TestClient, username: str, password: str) -> dict[str, str]:
    response = client.post(
        "/api/v1/auth/login",
        json={"username": username, "password": password},
    )
    assert response.status_code == 200
    return {"Authorization": f"Bearer {response.json()['access_token']}"}


def create_user(client: TestClient, username: str) -> tuple[dict[str, str], str]:
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
    return authenticate(client, username, "Strong123!"), str(response.json()["id"])


def test_complete_skillhive_workflow(client: TestClient, db_session: Session) -> None:
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
    admin_headers = authenticate(client, "admin", "Admin123!")
    owner_headers, _ = create_user(client, "owner")
    member_headers, _ = create_user(client, "member")

    private_skill = client.post(
        "/api/v1/skills",
        headers=owner_headers,
        json={
            "name": "My Planning Skill",
            "slug": "my-planning-skill",
            "content": {"instructions": "Create a focused weekly plan."},
        },
    )
    assert private_skill.status_code == 201

    group = client.post(
        "/api/v1/groups",
        headers=owner_headers,
        json={"name": "Delivery Team"},
    )
    group_id = str(group.json()["id"])
    invitation = client.post(
        f"/api/v1/groups/{group_id}/members/invite",
        headers=owner_headers,
        json={"identity": "member"},
    )
    assert (
        client.post(
            f"/api/v1/groups/invitations/{invitation.json()['id']}/accept",
            headers=member_headers,
        ).status_code
        == 200
    )

    global_skill = client.post(
        "/api/v1/admin/skills",
        headers=admin_headers,
        json={
            "name": "Team Planning",
            "slug": "team-planning",
            "content": {"instructions": "Plan team delivery milestones."},
            "version": "1.0.0",
        },
    )
    skill_id = str(global_skill.json()["id"])
    assert (
        client.post(
            f"/api/v1/admin/skills/{skill_id}/publish",
            headers=admin_headers,
            json={"version_id": global_skill.json()["current_version"]["id"]},
        ).status_code
        == 200
    )
    assert (
        client.post(
            f"/api/v1/groups/{group_id}/skills/{skill_id}",
            headers=owner_headers,
            json={"version_policy": "latest"},
        ).status_code
        == 201
    )
    member_view = client.get(
        f"/api/v1/groups/{group_id}/skills",
        headers=member_headers,
    )
    assert member_view.status_code == 200
    assert member_view.json()[0]["skill"]["name"] == "Team Planning"
    assert member_view.json()[0]["effective_version"]["version"] == "1.0.0"
