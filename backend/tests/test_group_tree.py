from typing import Any, cast

from app.core.security import hash_password
from app.models import User
from fastapi.testclient import TestClient
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


def build_tree(client: TestClient, owner_headers: dict[str, str]) -> tuple[str, str, str]:
    """Root A -> child B -> grandchild C, all managed by the same owner."""
    root = client.post("/api/v1/groups", headers=owner_headers, json={"name": "Root A"})
    assert root.status_code == 201
    root_id = str(root.json()["id"])
    child = client.post(
        "/api/v1/groups",
        headers=owner_headers,
        json={"name": "Child B", "parent_group_id": root_id},
    )
    assert child.status_code == 201
    child_id = str(child.json()["id"])
    assert child.json()["parent_id"] == root_id
    assert child.json()["parent_name"] == "Root A"
    grandchild = client.post(
        "/api/v1/groups",
        headers=owner_headers,
        json={"name": "Grandchild C", "parent_group_id": child_id},
    )
    assert grandchild.status_code == 201
    grandchild_id = str(grandchild.json()["id"])
    return root_id, child_id, grandchild_id


def test_subgroup_creation_requires_parent_manager(
    client: TestClient,
) -> None:
    owner_headers, _ = user_auth(client, "owner")
    outsider_headers, _ = user_auth(client, "outsider")
    root_id, _child_id, _grandchild_id = build_tree(client, owner_headers)

    # A root group can still be created by anyone.
    independent = client.post(
        "/api/v1/groups", headers=outsider_headers, json={"name": "Indie"}
    )
    assert independent.status_code == 201
    assert independent.json()["parent_id"] is None

    denied = client.post(
        "/api/v1/groups",
        headers=outsider_headers,
        json={"name": "Sneaky", "parent_group_id": root_id},
    )
    assert denied.status_code == 403
    assert denied.json()["error"]["code"] == "PERMISSION_DENIED"

    missing = client.post(
        "/api/v1/groups",
        headers=owner_headers,
        json={"name": "Orphan", "parent_group_id": "00000000-0000-0000-0000-000000000000"},
    )
    assert missing.status_code == 404


def test_inherited_manager_rights_on_descendants(
    client: TestClient,
) -> None:
    owner_headers, _owner_id = user_auth(client, "owner")
    admin_headers, admin_id = user_auth(client, "groupadmin")
    member_headers, member_id = user_auth(client, "member")
    root_id, child_id, grandchild_id = build_tree(client, owner_headers)

    # Owner invites the admin into the root and promotes them.
    invitation = client.post(
        f"/api/v1/groups/{root_id}/members/invite",
        headers=owner_headers,
        json={"identity": "groupadmin"},
    )
    assert invitation.status_code == 201
    assert (
        client.post(
            f"/api/v1/groups/invitations/{invitation.json()['id']}/accept",
            headers=admin_headers,
        ).status_code
        == 200
    )
    promoted = client.patch(
        f"/api/v1/groups/{root_id}/members/{admin_id}",
        headers=owner_headers,
        json={"role": "admin"},
    )
    assert promoted.status_code == 200

    # The root admin creates a sub-group and becomes its owner.
    sub = client.post(
        "/api/v1/groups",
        headers=admin_headers,
        json={"name": "Admin Sub", "parent_group_id": root_id},
    )
    assert sub.status_code == 201
    sub_id = str(sub.json()["id"])
    assert sub.json()["current_user_role"] == "owner"

    # The root owner is not a member of the sub-group but inherits management.
    viewed = client.get(f"/api/v1/groups/{sub_id}", headers=owner_headers)
    assert viewed.status_code == 200
    assert viewed.json()["current_user_role"] == "admin"
    assert client.get(f"/api/v1/groups/{sub_id}/members", headers=owner_headers).status_code == 200

    invitee = client.post(
        f"/api/v1/groups/{sub_id}/members/invite",
        headers=owner_headers,
        json={"identity": "member"},
    )
    assert invitee.status_code == 201
    assert (
        client.post(
            f"/api/v1/groups/invitations/{invitee.json()['id']}/accept",
            headers=member_headers,
        ).status_code
        == 200
    )

    # Inherited managers may remove plain members of the descendant.
    removed = client.delete(
        f"/api/v1/groups/{sub_id}/members/{member_id}",
        headers=owner_headers,
    )
    assert removed.status_code == 204

    # Owner-only operations do not travel down the tree.
    assert (
        client.delete(
            f"/api/v1/groups/{sub_id}/members/{admin_id}",
            headers=owner_headers,
        ).status_code
        == 403
    )
    denied_role = client.patch(
        f"/api/v1/groups/{sub_id}/members/{member_id}",
        headers=owner_headers,
        json={"role": "admin"},
    )
    assert denied_role.status_code == 403
    denied_dissolve = client.delete(f"/api/v1/groups/{sub_id}", headers=owner_headers)
    assert denied_dissolve.status_code == 403
    denied_transfer = client.post(
        f"/api/v1/groups/{sub_id}/transfer-ownership",
        headers=owner_headers,
        json={"new_owner_user_id": member_id},
    )
    assert denied_transfer.status_code == 403

    # A plain member of the root gets nothing on the descendant.
    plain_headers, _plain_id = user_auth(client, "plainmember")
    plain_invite = client.post(
        f"/api/v1/groups/{root_id}/members/invite",
        headers=owner_headers,
        json={"identity": "plainmember"},
    )
    assert plain_invite.status_code == 201
    assert (
        client.post(
            f"/api/v1/groups/invitations/{plain_invite.json()['id']}/accept",
            headers=plain_headers,
        ).status_code
        == 200
    )
    assert client.get(f"/api/v1/groups/{sub_id}", headers=plain_headers).status_code == 404
    assert client.get(f"/api/v1/groups/{sub_id}/members", headers=plain_headers).status_code == 404

    # Dissolving a parent with active children is refused.
    blocked = client.delete(f"/api/v1/groups/{root_id}", headers=owner_headers)
    assert blocked.status_code == 409
    assert blocked.json()["error"]["code"] == "GROUP_HAS_CHILDREN"

    # Once every child is dissolved (each by its own owner seat), the root
    # can be dissolved as well. The owner owns Child B / Grandchild C
    # directly; the admin owns Admin Sub.
    assert (
        client.delete(f"/api/v1/groups/{sub_id}", headers=admin_headers).status_code == 204
    )
    assert (
        client.delete(f"/api/v1/groups/{grandchild_id}", headers=owner_headers).status_code == 204
    )
    assert client.delete(f"/api/v1/groups/{child_id}", headers=owner_headers).status_code == 204
    assert client.delete(f"/api/v1/groups/{root_id}", headers=owner_headers).status_code == 204
    assert client.get(f"/api/v1/groups/{sub_id}", headers=admin_headers).status_code == 404


def test_move_reparent_cycle_and_depth(client: TestClient) -> None:
    owner_headers, _ = user_auth(client, "owner")
    root_id, child_id, grandchild_id = build_tree(client, owner_headers)

    self_parent = client.patch(
        f"/api/v1/groups/{root_id}",
        headers=owner_headers,
        json={"parent_group_id": root_id},
    )
    assert self_parent.status_code == 400
    assert self_parent.json()["error"]["code"] == "GROUP_CYCLE"

    under_own_subtree = client.patch(
        f"/api/v1/groups/{root_id}",
        headers=owner_headers,
        json={"parent_group_id": grandchild_id},
    )
    assert under_own_subtree.status_code == 400
    assert under_own_subtree.json()["error"]["code"] == "GROUP_CYCLE"

    # Reparenting requires manager rights on the target parent.
    outsider_headers, _ = user_auth(client, "outsider")
    outsider_group = client.post(
        "/api/v1/groups", headers=outsider_headers, json={"name": "Outsider Root"}
    )
    outsider_id = str(outsider_group.json()["id"])
    denied = client.patch(
        f"/api/v1/groups/{grandchild_id}",
        headers=owner_headers,
        json={"parent_group_id": outsider_id},
    )
    assert denied.status_code == 403

    # Move the grandchild to the root and back.
    to_root = client.patch(
        f"/api/v1/groups/{grandchild_id}",
        headers=owner_headers,
        json={"parent_group_id": None},
    )
    assert to_root.status_code == 200
    assert to_root.json()["parent_id"] is None
    reattach = client.patch(
        f"/api/v1/groups/{grandchild_id}",
        headers=owner_headers,
        json={"parent_group_id": child_id},
    )
    assert reattach.status_code == 200
    assert reattach.json()["parent_id"] == child_id

    # Depth limit: a chain of 16 levels is valid, the 17th is refused.
    parent_id = root_id
    for level in range(2, 17):
        created = client.post(
            "/api/v1/groups",
            headers=owner_headers,
            json={"name": f"Level {level}", "parent_group_id": parent_id},
        )
        assert created.status_code == 201, f"level {level} should be allowed"
        parent_id = str(created.json()["id"])
    too_deep = client.post(
        "/api/v1/groups",
        headers=owner_headers,
        json={"name": "Level 17", "parent_group_id": parent_id},
    )
    assert too_deep.status_code == 400
    assert too_deep.json()["error"]["code"] == "GROUP_DEPTH_EXCEEDED"


def test_tree_endpoint_visibility(client: TestClient, db_session: Session) -> None:
    owner_headers, _ = user_auth(client, "owner")
    member_headers, _ = user_auth(client, "member")
    root_id, child_id, grandchild_id = build_tree(client, owner_headers)

    invite = client.post(
        f"/api/v1/groups/{root_id}/members/invite",
        headers=owner_headers,
        json={"identity": "member"},
    )
    assert invite.status_code == 201
    assert (
        client.post(
            f"/api/v1/groups/invitations/{invite.json()['id']}/accept",
            headers=member_headers,
        ).status_code
        == 200
    )

    owner_tree = cast(
        list[dict[str, Any]],
        client.get("/api/v1/groups/tree", headers=owner_headers).json(),
    )
    owner_by_id = {str(group["id"]): group for group in owner_tree}
    assert set(owner_by_id) == {root_id, child_id, grandchild_id}
    # build_tree created all three groups with the same owner, so every node
    # reports its direct owner seat here (the inherited-admin display case is
    # covered by test_inherited_manager_rights_on_descendants).
    assert owner_by_id[root_id]["current_user_role"] == "owner"
    assert owner_by_id[child_id]["current_user_role"] == "owner"
    assert owner_by_id[grandchild_id]["current_user_role"] == "owner"

    member_tree = cast(
        list[dict[str, Any]], client.get("/api/v1/groups/tree", headers=member_headers).json()
    )
    assert [str(group["id"]) for group in member_tree] == [root_id]
    assert member_tree[0]["current_user_role"] == "member"

    admin_headers = global_admin_auth(client, db_session)
    admin_tree = cast(
        list[dict[str, Any]], client.get("/api/v1/groups/tree", headers=admin_headers).json()
    )
    assert {str(group["id"]) for group in admin_tree} == {root_id, child_id, grandchild_id}
    assert all(group["current_user_role"] == "owner" for group in admin_tree)

    # The global admin reads and manages groups they never joined.
    direct = client.get(f"/api/v1/groups/{child_id}", headers=admin_headers)
    assert direct.status_code == 200
    assert direct.json()["current_user_role"] == "owner"
    paged = client.get("/api/v1/groups", headers=admin_headers)
    assert paged.status_code == 200
    assert {str(item["id"]) for item in paged.json()["items"]} == {
        root_id,
        child_id,
        grandchild_id,
    }


def test_global_admin_owner_powers(client: TestClient, db_session: Session) -> None:
    owner_headers, owner_id = user_auth(client, "owner")
    member_headers, member_id = user_auth(client, "member")
    group = client.post("/api/v1/groups", headers=owner_headers, json={"name": "Managed"})
    group_id = str(group.json()["id"])
    invite = client.post(
        f"/api/v1/groups/{group_id}/members/invite",
        headers=owner_headers,
        json={"identity": "member"},
    )
    assert invite.status_code == 201
    assert (
        client.post(
            f"/api/v1/groups/invitations/{invite.json()['id']}/accept",
            headers=member_headers,
        ).status_code
        == 200
    )

    admin_headers = global_admin_auth(client, db_session)

    # Transfer ownership on a group the admin never joined.
    transferred = client.post(
        f"/api/v1/groups/{group_id}/transfer-ownership",
        headers=admin_headers,
        json={"new_owner_user_id": member_id},
    )
    assert transferred.status_code == 200

    role_changed = client.patch(
        f"/api/v1/groups/{group_id}/members/{owner_id}",
        headers=admin_headers,
        json={"role": "member"},
    )
    assert role_changed.status_code == 200

    # Dissolve a group the admin never joined.
    assert client.delete(f"/api/v1/groups/{group_id}", headers=admin_headers).status_code == 204
