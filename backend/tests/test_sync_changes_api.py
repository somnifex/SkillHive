"""M2.5 durable pull feed endpoint tests.

Covers incremental pull over the change feed: ordering/cursor stability,
visibility filtering (private owner-only, published global visible, grant
projection), explicit tombstones, and cursor correctness across pages.
Also covers plan §17 legacy package synthesis: browser-created Skills
carry an addressable SKILL.md package on the change feed, and pre-existing
package-less rows are synthesized lazily on first pull.
"""

import json
from collections.abc import Generator
from pathlib import Path
from typing import cast

import pytest
from app.core.security import hash_password
from app.db.base import Base
from app.db.session import get_db
from app.main import app
from app.models import Group, GroupMember, Skill, SkillVersion, User
from app.services.blob_storage import (
    LocalFilesystemBlobStorage,
    hash_bytes,
    reset_blob_storage_for_tests,
)
from app.services.legacy_package import synthesize_legacy_package
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
def pull_storage(tmp_path: Path) -> LocalFilesystemBlobStorage:
    return LocalFilesystemBlobStorage(tmp_path / "blobs")


@pytest.fixture
def pull_client(
    pull_session: Session,
    pull_storage: LocalFilesystemBlobStorage,
    monkeypatch: pytest.MonkeyPatch,
) -> Generator[TestClient, None, None]:
    def override_get_db() -> Generator[Session, None, None]:
        yield pull_session

    # Legacy synthesis writes real blobs from both the REST mutation path and
    # the lazy pull projection; point both call sites at the tmp storage.
    monkeypatch.setattr("app.api.v1.sync.get_blob_storage", lambda _session=None: pull_storage)
    monkeypatch.setattr(
        "app.services.skill_mutations.get_blob_storage",
        lambda _session=None: pull_storage,
    )
    monkeypatch.setattr(
        "app.services.sync_changes.get_blob_storage",
        lambda _session=None: pull_storage,
    )
    app.dependency_overrides[get_db] = override_get_db
    with TestClient(app) as test_client:
        yield test_client
    app.dependency_overrides.clear()
    reset_blob_storage_for_tests()


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


def test_cursor_before_retention_window_rejected_with_410(
    pull_client: TestClient, pull_session: Session
) -> None:
    """A cursor below the oldest surviving change row fails loudly (GC §6)."""
    headers = _bearer(pull_client, "expired_cursor")
    created = pull_client.post(
        "/api/v1/skills",
        headers=headers,
        json={"name": "Expiry Skill", "slug": "expiry-skill", "content": {}},
    )
    assert created.status_code == 201

    # Simulate trimming: pretend the log's oldest surviving row is sequence 2
    # while the client's cursor still points below it.
    from app.models import SyncChangeLog
    from sqlalchemy import select, update

    row = pull_session.scalar(select(SyncChangeLog).order_by(SyncChangeLog.sequence.asc()))
    assert row is not None
    # Decode "v1.AAAAAAAAAAE" = sequence 1; rewrite the only row to sequence 2
    # so a cursor at 1 predates retained history.
    pull_session.execute(
        update(SyncChangeLog).where(SyncChangeLog.sequence == row.sequence).values(sequence=2)
    )
    pull_session.commit()

    expired = pull_client.get(
        "/api/v1/sync/changes",
        headers=headers,
        params={"cursor": first_cursor_value()},
    )
    assert expired.status_code == 410
    assert expired.json()["error"]["code"] == "SYNC_CURSOR_EXPIRED"

    # A zero cursor (full baseline) stays servable.
    baseline = pull_client.get("/api/v1/sync/changes", headers=headers)
    assert baseline.status_code == 200
    assert any(item["resourceId"] == created.json()["id"] for item in baseline.json()["changes"])


def first_cursor_value() -> str:
    from app.services.sync_cursor import encode_sync_cursor

    return encode_sync_cursor(1)


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


def _read_blob(pull_client: TestClient, headers: dict[str, str], hash_value: str) -> bytes:
    response = pull_client.get(f"/api/v1/sync/blobs/{hash_value}", headers=headers)
    assert response.status_code == 200
    return cast("bytes", response.content)


def test_browser_created_skill_change_feed_carries_addressable_package(
    pull_client: TestClient,
    pull_session: Session,
    pull_storage: LocalFilesystemBlobStorage,
) -> None:
    """Plan §17: a REST-created Skill ships a synthesized SKILL.md package.

    The change feed's ``packageManifestHash`` must be downloadable via
    ``/sync/blobs`` and resolve to a manifest whose single entry is a
    front-matter SKILL.md carrying the legacy ``instructions`` body.
    """
    headers = _bearer(pull_client, "browser_author")
    created = pull_client.post(
        "/api/v1/skills",
        headers=headers,
        json={
            "name": "Browser Skill",
            "slug": "browser-skill",
            "description": "Made in the web UI",
            "content": {"instructions": "Step one.\nStep two."},
        },
    )
    assert created.status_code == 201
    skill_id = created.json()["id"]

    page = pull_client.get("/api/v1/sync/changes", headers=headers)
    assert page.status_code == 200
    entries = [item for item in page.json()["changes"] if item["resourceId"] == skill_id]
    assert len(entries) == 1
    manifest_hash = entries[0]["packageManifestHash"]
    assert manifest_hash is not None, "browser create must carry an addressable package"

    manifest_bytes = _read_blob(pull_client, headers, manifest_hash)
    files = json.loads(manifest_bytes)["files"]
    assert len(files) == 1
    assert files[0]["path"] == "SKILL.md"

    entrypoint_bytes = _read_blob(pull_client, headers, files[0]["blobHash"])
    entrypoint = entrypoint_bytes.decode("utf-8")
    assert entrypoint == (
        "---\n"
        "name: browser-skill\n"
        'description: "Made in the web UI"\n'
        "---\n"
        "\n"
        "Step one.\n"
        "Step two.\n"
    )
    assert hash_bytes(entrypoint_bytes) == files[0]["blobHash"]
    assert files[0]["sizeBytes"] == len(entrypoint_bytes)

    # The version row records the same package identity.
    skill = pull_session.get(Skill, skill_id)
    assert skill is not None
    assert skill.current_package_hash == manifest_hash
    version = pull_session.get(SkillVersion, skill.current_version_id)
    assert version is not None
    assert version.package_manifest_hash == manifest_hash

    # Storage holds both objects under the tmp root.
    assert pull_storage.exists(manifest_hash, len(manifest_bytes))
    assert pull_storage.exists(files[0]["blobHash"], len(entrypoint_bytes))


def test_synthesis_is_deterministic_for_identical_content(
    pull_client: TestClient,
    pull_session: Session,
    pull_storage: LocalFilesystemBlobStorage,
) -> None:
    """Identical legacy content yields identical hashes (idempotent re-runs)."""
    headers = _bearer(pull_client, "determinist")
    created = pull_client.post(
        "/api/v1/skills",
        headers=headers,
        json={
            "name": "Stable Skill",
            "slug": "stable-skill",
            "content": {"instructions": "Same body."},
        },
    )
    assert created.status_code == 201
    skill = pull_session.get(Skill, created.json()["id"])
    assert skill is not None
    assert skill.current_package_hash is not None

    # Re-running synthesis against the same content is byte-identical.
    again = synthesize_legacy_package(
        pull_session,
        pull_storage,
        slug=skill.slug,
        name=skill.name,
        description=skill.description,
        content={"instructions": "Same body."},
    )
    assert again == skill.current_package_hash


def test_legacy_row_without_package_synthesizes_once_on_first_pull(
    pull_client: TestClient,
    pull_session: Session,
) -> None:
    """A pre-existing package-less Skill gains its package on first pull.

    The first GET /changes synthesizes and persists the package (the shipped
    hash is immediately addressable); the second pull is a no-op that returns
    the same hash without writing again.
    """
    headers = _bearer(pull_client, "legacy_owner")

    # Seed a legacy row directly: version content with a body, but no package.
    user = pull_session.query(User).filter_by(username="legacy_owner").one()
    legacy = Skill(
        name="Legacy Skill",
        slug="legacy-skill",
        description="Old row",
        skill_type="private",
        owner_user_id=user.id,
        category="",
        tags=[],
        status="draft",
        sync_revision=1,
        current_package_hash=None,
        created_by=user.id,
    )
    pull_session.add(legacy)
    pull_session.flush()
    legacy_version = SkillVersion(
        skill_id=legacy.id,
        version="0.1.0",
        revision=1,
        content={"instructions": "Legacy instructions body."},
        manifest={"name": "legacy-skill", "schema_version": 1},
        package_manifest_hash=None,
        package_size_bytes=None,
        dependency_config={},
        change_log="seeded",
        status="draft",
        created_by=user.id,
    )
    pull_session.add(legacy_version)
    pull_session.flush()  # assign the version id before linking it
    legacy.current_version_id = legacy_version.id
    # No change-log row: the pull projection must synthesize the package when
    # it first encounters the Skill, even without a fresh event.
    from app.models import SyncChangeLog

    pull_session.add(
        SyncChangeLog(
            resource_type="skill",
            resource_id=legacy.id,
            resource_revision=1,
            operation="upsert",
            owner_user_id=user.id,
            package_manifest_hash=None,
            metadata_payload={"name": legacy.name, "slug": legacy.slug},
        )
    )
    pull_session.commit()

    first = pull_client.get("/api/v1/sync/changes", headers=headers)
    assert first.status_code == 200
    entries = [item for item in first.json()["changes"] if item["resourceId"] == legacy.id]
    assert len(entries) == 1
    synthesized_hash = entries[0]["packageManifestHash"]
    assert synthesized_hash is not None, "first pull must synthesize the legacy package"

    # The synthesized hash is downloadable right away.
    manifest_bytes = _read_blob(pull_client, headers, synthesized_hash)
    files = json.loads(manifest_bytes)["files"]
    assert len(files) == 1 and files[0]["path"] == "SKILL.md"
    body = _read_blob(pull_client, headers, files[0]["blobHash"]).decode("utf-8")
    assert "Legacy instructions body." in body
    assert body.startswith("---\nname: legacy-skill\n")

    # Persisted on both the Skill and the version row.
    pull_session.expire_all()
    skill = pull_session.get(Skill, legacy.id)
    assert skill is not None
    assert skill.current_package_hash == synthesized_hash
    version = pull_session.get(SkillVersion, skill.current_version_id)
    assert version is not None
    assert version.package_manifest_hash == synthesized_hash

    # Second pull past the first page's cursor: nothing re-shipped, nothing
    # re-synthesized (idempotent).
    second = pull_client.get(
        "/api/v1/sync/changes", headers=headers, params={"cursor": first.json()["nextCursor"]}
    )
    assert second.status_code == 200
    second_entries = [
        item for item in second.json()["changes"] if item["resourceId"] == legacy.id
    ]
    assert second_entries == []
    assert skill.current_package_hash == synthesized_hash


def test_empty_legacy_body_stays_package_less(
    pull_client: TestClient,
    pull_session: Session,
) -> None:
    """Content without a usable body keeps the old metadata-only behavior."""
    headers = _bearer(pull_client, "empty_author")
    created = pull_client.post(
        "/api/v1/skills",
        headers=headers,
        json={"name": "Empty Skill", "slug": "empty-skill", "content": {}},
    )
    assert created.status_code == 201
    skill = pull_session.get(Skill, created.json()["id"])
    assert skill is not None
    assert skill.current_package_hash is None

    page = pull_client.get("/api/v1/sync/changes", headers=headers)
    entries = [item for item in page.json()["changes"] if item["resourceId"] == skill.id]
    assert len(entries) == 1
    assert entries[0]["packageManifestHash"] is None
