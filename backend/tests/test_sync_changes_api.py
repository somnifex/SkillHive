"""M2.5 durable pull feed endpoint tests.

Covers incremental pull over the change feed: ordering/cursor stability,
visibility filtering (private owner-only, published global visible, grant
projection), explicit tombstones, and cursor correctness across pages.
"""

from collections.abc import Generator
from pathlib import Path

import pytest
from app.core.security import hash_password
from app.db.base import Base
from app.db.session import get_db
from app.main import app
from app.models import Group, GroupMember, Skill, User
from fastapi.testclient import TestClient
from sqlalchemy import create_engine
from sqlalchemy.orm import Session, sessionmaker


@pytest.fixture
def pull_session(tmp_path: Path) -> Generator[Session, None, None]:
    engine = create_engine(
        f"sqlite:///{(tmp_path / 'pull.db').as_posix()}",
        connect_args={"check_same_thread": False},
    )
    Base.metadata.create_all(engine)
    factory = sessionmaker(bind=engine, autoflush=False, expire_on_commit=False)
    with factory() as session:
        yield session
    engine.dispose()


@pytest.fixture
def pull_client(pull_session: Session) -> Generator[TestClient, None, None]:
    def override_get_db() -> Generator[Session, None, None]:
        yield pull_session

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
    return {"Authorization": f"Bearer {login.json()['access_token']}"}


def test_private_create_and_delete_reach_pull_as_upsert_and_tombstone(
    pull_client: TestClient,
) -> None:
    headers = _bearer(pull_client, "pull_owner")

    created = pull_client.post(
        "/api/v1/skills",
        headers=headers,
        json={"name": "Pull Skill", "slug": "pull-skill", "content": {}},
    )
    assert created.status_code == 201
    skill_id = created.json()["id"]

    first_page = pull_client.get("/api/v1/sync/changes", headers=headers)
    assert first_page.status_code == 200
    entries = first_page.json()["changes"]
    assert [item["operation"] for item in entries] == ["upsert"]
    assert entries[0]["resourceId"] == skill_id
    assert entries[0]["resourceRevision"] == 1
    assert entries[0]["metadata"]["name"] == "Pull Skill"

    deleted = pull_client.delete(f"/api/v1/skills/{skill_id}", headers=headers)
    assert deleted.status_code == 204

    after = pull_client.get(
        "/api/v1/sync/changes",
        headers=headers,
        params={"cursor": first_page.json()["nextCursor"]},
    )
    assert after.status_code == 200
    tombstones = [item for item in after.json()["changes"] if item["resourceId"] == skill_id]
    assert [item["operation"] for item in tombstones] == ["delete"]
    assert tombstones[0]["resourceRevision"] == 2


def test_pull_pages_are_ordered_and_cursor_resumes(
    pull_client: TestClient,
) -> None:
    headers = _bearer(pull_client, "pager")
    for index in range(5):
        created = pull_client.post(
            "/api/v1/skills",
            headers=headers,
            json={"name": f"Skill {index}", "slug": f"skill-{index}", "content": {}},
        )
        assert created.status_code == 201

    first = pull_client.get("/api/v1/sync/changes", headers=headers, params={"limit": 2})
    assert first.status_code == 200
    body = first.json()
    assert len(body["changes"]) == 2
    assert body["hasMore"] is True
    sequences = [item["sequence"] for item in body["changes"]]
    assert sequences == sorted(sequences)

    second = pull_client.get(
        "/api/v1/sync/changes", headers=headers, params={"cursor": body["nextCursor"], "limit": 2}
    )
    second_sequences = [item["sequence"] for item in second.json()["changes"]]
    assert second_sequences[0] > sequences[-1]
    assert second.json()["hasMore"] is True


def test_foreign_private_skill_not_visible(
    pull_client: TestClient,
) -> None:
    owner = _bearer(pull_client, "private_owner")
    outsider = _bearer(pull_client, "outsider")

    created = pull_client.post(
        "/api/v1/skills",
        headers=owner,
        json={"name": "Secret", "slug": "secret-skill", "content": {}},
    )
    assert created.status_code == 201

    outsider_view = pull_client.get("/api/v1/sync/changes", headers=outsider)
    assert outsider_view.status_code == 200
    assert outsider_view.json()["changes"] == []
    assert outsider_view.json()["hasMore"] is False

    owner_view = pull_client.get("/api/v1/sync/changes", headers=owner)
    assert len(owner_view.json()["changes"]) == 1


def test_global_skill_visible_only_when_published(
    pull_client: TestClient,
    pull_session: Session,
) -> None:
    admin = User(
        username="global_admin",
        display_name="Admin",
        email="global_admin@example.com",
        password_hash=hash_password("Admin123!"),
        status="active",
        is_global_admin=True,
    )
    pull_session.add(admin)
    pull_session.commit()

    viewer = _bearer(pull_client, "global_viewer")
    admin_headers = {
        "Authorization": "Bearer "
        + pull_client.post(
            "/api/v1/auth/login",
            json={"username": "global_admin", "password": "Admin123!"},
        ).json()["access_token"]
    }

    created = pull_client.post(
        "/api/v1/admin/skills",
        headers=admin_headers,
        json={"name": "Team Skill", "slug": "team-skill", "content": {}},
    )
    assert created.status_code == 201
    skill_id = created.json()["id"]

    draft_view = pull_client.get("/api/v1/sync/changes", headers=viewer)
    assert [item["resourceId"] for item in draft_view.json()["changes"]] == []

    published = pull_client.post(
        f"/api/v1/admin/skills/{skill_id}/publish", headers=admin_headers, json={}
    )
    assert published.status_code == 200

    published_view = pull_client.get(
        "/api/v1/sync/changes",
        headers=viewer,
        params={"cursor": draft_view.json()["nextCursor"]},
    )
    entries = [item for item in published_view.json()["changes"] if item["resourceId"] == skill_id]
    assert [item["operation"] for item in entries] == ["upsert"]
    assert entries[0]["resourceRevision"] == 2


def test_group_grant_entitles_pull_and_tombstone_stays_visible(
    pull_client: TestClient,
    pull_session: Session,
) -> None:
    admin = User(
        username="grant_admin",
        display_name="Grant Admin",
        email="grant_admin@example.com",
        password_hash=hash_password("Admin123!"),
        status="active",
        is_global_admin=True,
    )
    member = User(
        username="grant_member",
        display_name="Grant Member",
        email="grant_member@example.com",
        password_hash=hash_password("User123!"),
        status="active",
    )
    pull_session.add_all([admin, member])
    pull_session.commit()

    group = Group(
        name="Grant Group",
        description="",
        group_type="platform",
        owner_id=admin.id,
        created_by=admin.id,
        status="active",
    )
    pull_session.add(group)
    pull_session.flush()
    # The owner membership is what makes the admin a group manager.
    pull_session.add(
        GroupMember(group_id=group.id, user_id=admin.id, role="owner", status="active")
    )
    pull_session.add(
        GroupMember(group_id=group.id, user_id=member.id, role="member", status="active")
    )
    pull_session.commit()

    member_headers = {
        "Authorization": "Bearer "
        + pull_client.post(
            "/api/v1/auth/login",
            json={"username": "grant_member", "password": "User123!"},
        ).json()["access_token"]
    }
    admin_headers = {
        "Authorization": "Bearer "
        + pull_client.post(
            "/api/v1/auth/login",
            json={"username": "grant_admin", "password": "Admin123!"},
        ).json()["access_token"]
    }

    created = pull_client.post(
        "/api/v1/admin/skills",
        headers=admin_headers,
        json={"name": "Granted Skill", "slug": "granted-skill", "content": {}},
    )
    assert created.status_code == 201
    skill_id = created.json()["id"]
    assert (
        pull_client.post(
            f"/api/v1/admin/skills/{skill_id}/publish", headers=admin_headers, json={}
        ).status_code
        == 200
    )
    assert (
        pull_client.post(
            f"/api/v1/groups/{group.id}/skills/{skill_id}",
            headers=admin_headers,
            json={"version_policy": "latest"},
        ).status_code
        == 201
    )

    view = pull_client.get("/api/v1/sync/changes", headers=member_headers)
    entries = [item for item in view.json()["changes"] if item["resourceId"] == skill_id]
    assert entries, "grant+membership projection should surface the granted global skill"

    revoked = pull_client.delete(
        f"/api/v1/admin/skills/{skill_id}/grants/{group.id}", headers=admin_headers
    )
    assert revoked.status_code == 204

    after_revoke = pull_client.get(
        "/api/v1/sync/changes",
        headers=member_headers,
        params={"cursor": view.json()["nextCursor"]},
    )
    # Published global Skills remain pull-visible; grant state gates M3 leases,
    # not the M2 read projection for published content.
    assert after_revoke.status_code == 200


def test_malformed_cursor_rejected(pull_client: TestClient) -> None:
    headers = _bearer(pull_client, "cursor_tester")
    response = pull_client.get(
        "/api/v1/sync/changes", headers=headers, params={"cursor": "v2.bogus"}
    )
    assert response.status_code == 400
    assert response.json()["error"]["code"] == "SYNC_CURSOR_INVALID"


def test_skill_model_revision_visible_in_read_model(
    pull_client: TestClient, pull_session: Session
) -> None:
    headers = _bearer(pull_client, "revision_viewer")
    created = pull_client.post(
        "/api/v1/skills",
        headers=headers,
        json={"name": "Revision Skill", "slug": "revision-skill", "content": {}},
    )
    assert created.status_code == 201
    skill = pull_session.get(Skill, created.json()["id"])
    assert skill is not None
    assert skill.sync_revision == 1
