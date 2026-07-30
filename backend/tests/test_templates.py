from typing import Any, cast

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


def invite_and_accept(
    client: TestClient,
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


def test_registration_adds_openai_default_and_instantiates_private_skill(
    client: TestClient,
) -> None:
    headers, _ = user_auth(client, "templateuser")
    listing = client.get("/api/v1/templates", headers=headers)
    assert listing.status_code == 200
    templates = cast(list[dict[str, Any]], listing.json()["items"])
    assert len(templates) == 1
    default = templates[0]
    assert default["is_default"] is True
    assert default["scope_type"] == "personal"
    assert default["manifest"]["entrypoint"] == "SKILL.md"
    assert default["manifest"]["required_frontmatter"] == ["name", "description"]

    created = client.post(
        f"/api/v1/templates/{default['id']}/instantiate",
        headers=headers,
        json={
            "name": "会议行动项",
            "slug": "meeting-actions",
            "description": "提取会议中的行动项和责任人。",
            "instructions": "提取行动项、负责人和截止日期。",
        },
    )
    assert created.status_code == 201
    skill = created.json()
    assert skill["skill_type"] == "private"
    assert skill["current_version"]["manifest"]["source_template_id"] == default["id"]
    markdown = skill["current_version"]["content"]["skill_markdown"]
    assert "name: meeting-actions" in markdown
    assert 'description: "提取会议中的行动项和责任人。"' in markdown
    assert "提取行动项、负责人和截止日期。" in markdown
    assert client.delete(f"/api/v1/templates/{default['id']}", headers=headers).status_code == 409


def test_group_and_global_template_permissions(
    client: TestClient,
    db_session: Session,
) -> None:
    owner_headers, _ = user_auth(client, "templateowner")
    member_headers, _ = user_auth(client, "templatemember")
    outsider_headers, _ = user_auth(client, "templateoutsider")
    admin_headers, admin_id = user_auth(client, "templateadmin")
    admin = db_session.get(User, admin_id)
    assert admin is not None
    admin.is_global_admin = True
    db_session.commit()

    group_response = client.post(
        "/api/v1/groups",
        headers=owner_headers,
        json={"name": "Template Team"},
    )
    group_id = str(group_response.json()["id"])
    invite_and_accept(
        client,
        group_id,
        owner_headers,
        member_headers,
        "templatemember",
    )

    group_template = client.post(
        "/api/v1/templates",
        headers=owner_headers,
        json={
            "name": "团队评审模板",
            "slug": "team-review",
            "description": "供团队成员进行结构化评审。",
            "scope_type": "group",
            "group_id": group_id,
            "content": {"instructions": "按正确性、风险和建议输出评审。"},
        },
    )
    assert group_template.status_code == 201
    group_template_id = str(group_template.json()["id"])

    member_listing = client.get("/api/v1/templates?scope_type=group", headers=member_headers)
    assert [item["id"] for item in member_listing.json()["items"]] == [group_template_id]
    assert member_listing.json()["items"][0]["can_manage"] is False
    assert (
        client.patch(
            f"/api/v1/templates/{group_template_id}",
            headers=member_headers,
            json={"name": "越权修改"},
        ).status_code
        == 403
    )
    assert (
        client.get(f"/api/v1/templates/{group_template_id}", headers=outsider_headers).status_code
        == 404
    )
    assert (
        client.post(
            "/api/v1/templates",
            headers=member_headers,
            json={
                "name": "越权群组模板",
                "slug": "unauthorized-group-template",
                "scope_type": "group",
                "group_id": group_id,
            },
        ).status_code
        == 403
    )

    assert (
        client.post(
            "/api/v1/templates",
            headers=owner_headers,
            json={
                "name": "越权全局模板",
                "slug": "unauthorized-global-template",
                "scope_type": "global",
            },
        ).status_code
        == 403
    )
    global_template = client.post(
        "/api/v1/templates",
        headers=admin_headers,
        json={
            "name": "全局需求模板",
            "slug": "global-requirement",
            "scope_type": "global",
            "content": {"instructions": "整理目标、范围、约束和验收标准。"},
        },
    )
    assert global_template.status_code == 201
    global_template_id = str(global_template.json()["id"])
    visible_to_outsider = client.get(
        "/api/v1/templates?scope_type=global",
        headers=outsider_headers,
    )
    assert [item["id"] for item in visible_to_outsider.json()["items"]] == [global_template_id]

    personal_template = client.post(
        "/api/v1/templates",
        headers=owner_headers,
        json={
            "name": "私人模板",
            "slug": "private-template",
            "scope_type": "personal",
        },
    )
    personal_id = str(personal_template.json()["id"])
    admin_listing = client.get("/api/v1/templates", headers=admin_headers)
    assert personal_id not in [item["id"] for item in admin_listing.json()["items"]]
    assert client.get(f"/api/v1/templates/{personal_id}", headers=admin_headers).status_code == 404
