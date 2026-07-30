from typing import Any, cast

from app.models import Group, GroupMember
from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session


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


def test_group_member_admin_and_owner_permissions(
    client: TestClient,
    db_session: Session,
) -> None:
    owner_headers, owner_id = user_auth(client, "owner")
    admin_headers, admin_id = user_auth(client, "groupadmin")
    member_headers, member_id = user_auth(client, "member")

    created = client.post(
        "/api/v1/groups",
        headers=owner_headers,
        json={"name": "Engineering", "join_policy": "invite_only"},
    )
    assert created.status_code == 201
    group = cast(dict[str, Any], created.json())
    group_id = str(group["id"])
    assert group["current_user_role"] == "owner"

    owner_memberships = list(
        db_session.scalars(
            select(GroupMember).where(
                GroupMember.group_id == group_id,
                GroupMember.role == "owner",
            )
        )
    )
    assert len(owner_memberships) == 1

    admin_invite = client.post(
        f"/api/v1/groups/{group_id}/members/invite",
        headers=owner_headers,
        json={"identity": "groupadmin"},
    )
    assert admin_invite.status_code == 201
    invitation_id = admin_invite.json()["id"]
    assert (
        client.post(
            f"/api/v1/groups/invitations/{invitation_id}/accept",
            headers=admin_headers,
        ).status_code
        == 200
    )
    promoted = client.patch(
        f"/api/v1/groups/{group_id}/members/{admin_id}",
        headers=owner_headers,
        json={"role": "admin"},
    )
    assert promoted.status_code == 200
    assert promoted.json()["role"] == "admin"

    updated = client.patch(
        f"/api/v1/groups/{group_id}",
        headers=admin_headers,
        json={"name": "Engineering Guild", "join_policy": "public"},
    )
    assert updated.status_code == 200
    assert updated.json()["name"] == "Engineering Guild"
    assert updated.json()["join_policy"] == "invite_only"

    member_invite = client.post(
        f"/api/v1/groups/{group_id}/members/invite",
        headers=admin_headers,
        json={"identity": "member"},
    )
    assert member_invite.status_code == 201
    assert (
        client.post(
            f"/api/v1/groups/invitations/{member_invite.json()['id']}/accept",
            headers=member_headers,
        ).status_code
        == 200
    )

    assert (
        client.delete(
            f"/api/v1/groups/{group_id}/members/{owner_id}",
            headers=admin_headers,
        ).status_code
        == 403
    )
    assert (
        client.delete(
            f"/api/v1/groups/{group_id}/members/{member_id}",
            headers=admin_headers,
        ).status_code
        == 204
    )
    assert client.post(f"/api/v1/groups/{group_id}/leave", headers=owner_headers).status_code == 409

    transferred = client.post(
        f"/api/v1/groups/{group_id}/transfer-ownership",
        headers=owner_headers,
        json={"new_owner_user_id": admin_id},
    )
    assert transferred.status_code == 200
    db_session.expire_all()
    stored_group = db_session.get(Group, group_id)
    assert stored_group is not None and stored_group.owner_id == admin_id
    owner_rows = list(
        db_session.scalars(
            select(GroupMember).where(
                GroupMember.group_id == group_id,
                GroupMember.role == "owner",
                GroupMember.status == "active",
            )
        )
    )
    assert len(owner_rows) == 1 and owner_rows[0].user_id == admin_id

    assert client.delete(f"/api/v1/groups/{group_id}", headers=owner_headers).status_code == 403
    assert client.delete(f"/api/v1/groups/{group_id}", headers=admin_headers).status_code == 204


def test_group_join_request_flow(client: TestClient) -> None:
    owner_headers, _ = user_auth(client, "owner")
    applicant_headers, applicant_id = user_auth(client, "applicant")
    created = client.post(
        "/api/v1/groups",
        headers=owner_headers,
        json={"name": "Research", "join_policy": "approval_required"},
    )
    group_id = created.json()["id"]

    request = client.post(
        f"/api/v1/groups/{group_id}/join-requests",
        headers=applicant_headers,
        json={"message": "I work on this topic."},
    )
    assert request.status_code == 200
    request_id = request.json()["id"]

    listing = client.get(
        f"/api/v1/groups/{group_id}/join-requests",
        headers=owner_headers,
    )
    assert listing.status_code == 200
    assert listing.json()[0]["user_id"] == applicant_id

    approved = client.patch(
        f"/api/v1/groups/{group_id}/join-requests/{request_id}",
        headers=owner_headers,
        json={"decision": "approved"},
    )
    assert approved.status_code == 200
    assert client.get(f"/api/v1/groups/{group_id}", headers=applicant_headers).status_code == 200
