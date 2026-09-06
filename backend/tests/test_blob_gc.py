"""Server blob GC tests (M2.2 destructive sweep, plan §9.7 / GC_DESIGN.md).

Covers the mark-and-sweep contract over real SQLite + tmp filesystem
storage: live packages survive, orphans are collected, in-flight uploads
are protected by the orphan grace, change-log/receipt roots hold, closure
expansion keeps manifest + files together, legal pins work, the batch
bound converges, and the run is idempotent.
"""

from __future__ import annotations

import hashlib
import json
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import pytest
from app.db.base import Base
from app.models import (
    Skill,
    SkillBlobObject,
    SkillVersion,
    SyncChangeLog,
    SyncMutationReceipt,
    User,
)
from app.services.blob_gc import run_blob_gc
from app.services.blob_registry import register_verified_blob
from app.services.blob_storage import LocalFilesystemBlobStorage
from sqlalchemy import create_engine
from sqlalchemy.orm import Session, sessionmaker

NOW = datetime(2026, 9, 6, 12, 0, 0, tzinfo=UTC)


@pytest.fixture
def gc_session(tmp_path: Path) -> Session:
    engine = create_engine(
        f"sqlite:///{(tmp_path / 'gc.db').as_posix()}",
        connect_args={"check_same_thread": False},
    )
    Base.metadata.create_all(engine)
    factory = sessionmaker(bind=engine, autoflush=False, expire_on_commit=False)
    session = factory()
    yield session
    session.close()
    engine.dispose()


@pytest.fixture
def gc_storage(tmp_path: Path) -> LocalFilesystemBlobStorage:
    return LocalFilesystemBlobStorage(tmp_path / "blobs")


def _hash(payload: bytes) -> str:
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def _put(
    session: Session,
    storage: LocalFilesystemBlobStorage,
    payload: bytes,
    *,
    created_at: datetime | None = None,
) -> str:
    """Register a blob, optionally backdating its registry row."""
    digest = _hash(payload)
    register_verified_blob(session, storage, digest, payload)
    if created_at is not None:
        row = session.get(SkillBlobObject, digest)
        assert row is not None
        row.created_at = created_at
        session.flush()
    return digest


def _manifest_payload(file_hashes: list[str]) -> bytes:
    files = [
        {"path": f"file-{index}.md", "blobHash": digest, "sizeBytes": 10}
        for index, digest in enumerate(file_hashes)
    ]
    # A manifest with one SKILL.md entrypoint plus listed files.
    files.insert(0, {"path": "SKILL.md", "blobHash": file_hashes[0], "sizeBytes": 10})
    return json.dumps({"format_version": 1, "files": files}).encode("utf-8")


def _seed_user(session: Session, username: str = "gc_user") -> User:
    user = User(
        username=username,
        display_name=username,
        email=f"{username}@example.com",
        password_hash="x",
        status="active",
    )
    session.add(user)
    session.flush()
    return user


def _seed_skill_with_version(
    session: Session,
    user: User,
    manifest_hash: str,
    *,
    revision: int = 1,
) -> tuple[Skill, SkillVersion]:
    skill = Skill(
        name="GC Skill",
        slug="gc-skill",
        description="",
        skill_type="private",
        owner_user_id=user.id,
        category="",
        tags=[],
        status="draft",
        sync_revision=revision,
        current_package_hash=manifest_hash,
        created_by=user.id,
    )
    session.add(skill)
    session.flush()
    version = SkillVersion(
        skill_id=skill.id,
        version="0.1.0",
        revision=revision,
        content={},
        manifest={},
        package_manifest_hash=manifest_hash,
        package_size_bytes=None,
        dependency_config={},
        change_log="",
        status="draft",
        created_by=user.id,
    )
    session.add(version)
    session.flush()
    skill.current_version_id = version.id
    session.flush()
    return skill, version


def _run(
    session: Session,
    storage: LocalFilesystemBlobStorage,
    **overrides: Any,
):
    kwargs: dict[str, Any] = {
        "now": NOW,
        "orphan_grace": timedelta(hours=24),
        "change_retention": timedelta(days=90),
        "receipt_retention": timedelta(days=90),
    }
    kwargs.update(overrides)
    return run_blob_gc(session, storage, **kwargs)


def _registered_hashes(session: Session) -> set[str]:
    return set(session.scalars(__import__("sqlalchemy").select(SkillBlobObject.hash)))


def test_live_package_and_closure_survive_orphan_swept(
    gc_session: Session, gc_storage: LocalFilesystemBlobStorage
) -> None:
    user = _seed_user(gc_session)
    file_blob = b"x" * 10
    manifest = _manifest_payload([_hash(file_blob)])
    manifest_hash = _put(gc_session, gc_storage, manifest, created_at=NOW - timedelta(days=7))
    file_hash = _put(gc_session, gc_storage, file_blob, created_at=NOW - timedelta(days=7))
    _seed_skill_with_version(gc_session, user, manifest_hash)

    # An unreferenced old object should be collected.
    orphan = _put(gc_session, gc_storage, b"orphan", created_at=NOW - timedelta(days=7))
    gc_session.commit()

    report = _run(gc_session, gc_storage)
    gc_session.commit()

    assert orphan not in _registered_hashes(gc_session)
    assert manifest_hash in _registered_hashes(gc_session)
    assert file_hash in _registered_hashes(gc_session)
    assert report.deleted_count == 1
    assert not gc_storage.exists(orphan, len(b"orphan"))
    assert gc_storage.exists(manifest_hash, len(manifest))
    assert gc_storage.exists(file_hash, len(file_blob))


def test_orphan_grace_protects_in_flight_uploads(
    gc_session: Session, gc_storage: LocalFilesystemBlobStorage
) -> None:
    _seed_user(gc_session)
    recent_orphan = _put(
        gc_session, gc_storage, b"in-flight", created_at=NOW - timedelta(hours=1)
    )
    gc_session.commit()

    _run(gc_session, gc_storage)
    gc_session.commit()

    # Created inside the 24h grace: kept even though unreferenced (R4).
    assert recent_orphan in _registered_hashes(gc_session)


def test_change_log_root_holds_retained_rows_only(
    gc_session: Session, gc_storage: LocalFilesystemBlobStorage
) -> None:
    user = _seed_user(gc_session)
    recent_blob = _put(
        gc_session, gc_storage, b"feed-content", created_at=NOW - timedelta(days=7)
    )
    # Referenced only by a change-log row inside the retention window.
    gc_session.add(
        SyncChangeLog(
            resource_type="skill",
            resource_id="00000000-0000-0000-0000-000000000001",
            resource_revision=1,
            operation="upsert",
            owner_user_id=user.id,
            package_manifest_hash=recent_blob,
            metadata_payload={},
            created_at=NOW - timedelta(days=7),
        )
    )
    gc_session.flush()
    del user  # the row's owner id is set; the user row itself need not persist
    gc_session.commit()

    _run(gc_session, gc_storage)
    gc_session.commit()

    assert recent_blob in _registered_hashes(gc_session)


def test_expired_change_log_row_releases_its_blobs(
    gc_session: Session, gc_storage: LocalFilesystemBlobStorage
) -> None:
    _seed_user(gc_session)
    stale_blob = _put(
        gc_session, gc_storage, b"expired-feed", created_at=NOW - timedelta(days=200)
    )
    gc_session.add(
        SyncChangeLog(
            resource_type="skill",
            resource_id="00000000-0000-0000-0000-000000000002",
            resource_revision=1,
            operation="upsert",
            owner_user_id=None,
            package_manifest_hash=stale_blob,
            metadata_payload={},
            created_at=NOW - timedelta(days=180),
        )
    )
    gc_session.commit()

    _run(gc_session, gc_storage)
    gc_session.commit()

    # Older than the 90-day retention window: the row no longer roots it.
    assert stale_blob not in _registered_hashes(gc_session)


def test_unexpired_receipt_root_holds_manifest(
    gc_session: Session, gc_storage: LocalFilesystemBlobStorage
) -> None:
    _seed_user(gc_session)
    manifest_blob = _put(
        gc_session, gc_storage, b"receipt-manifest", created_at=NOW - timedelta(days=7)
    )
    gc_session.add(
        SyncMutationReceipt(
            user_id="00000000-0000-0000-0000-000000000002",
            device_id="00000000-0000-0000-0000-000000000003",
            mutation_id="m-1",
            operation="create",
            resource_type="skill",
            resource_id=None,
            result_code="acked",
            result_revision=1,
            response_payload={
                "mutationId": "m-1",
                "status": "acked",
                "result": {
                    "remoteSkillId": "00000000-0000-0000-0000-000000000004",
                    "revision": 1,
                    "packageManifestHash": manifest_blob,
                },
            },
            created_at=NOW - timedelta(days=7),
        )
    )
    gc_session.commit()

    _run(gc_session, gc_storage)
    gc_session.commit()

    assert manifest_blob in _registered_hashes(gc_session)


def test_legal_roots_pin_objects(
    gc_session: Session, gc_storage: LocalFilesystemBlobStorage
) -> None:
    _seed_user(gc_session)
    pinned = _put(gc_session, gc_storage, b"legal-hold", created_at=NOW - timedelta(days=30))
    gc_session.commit()

    _run(gc_session, gc_storage, legal_roots={pinned})
    gc_session.commit()

    assert pinned in _registered_hashes(gc_session)


def test_closure_expansion_keeps_manifest_and_files_together(
    gc_session: Session, gc_storage: LocalFilesystemBlobStorage
) -> None:
    user = _seed_user(gc_session)
    file_blob = b"f" * 10
    file_hash = _put(gc_session, gc_storage, file_blob, created_at=NOW - timedelta(days=7))
    manifest = _manifest_payload([file_hash])
    manifest_hash = _put(gc_session, gc_storage, manifest, created_at=NOW - timedelta(days=7))
    _seed_skill_with_version(gc_session, user, manifest_hash)
    gc_session.commit()

    _run(gc_session, gc_storage)
    gc_session.commit()

    # Both survive: the rooted manifest keeps its closure.
    assert manifest_hash in _registered_hashes(gc_session)
    assert file_hash in _registered_hashes(gc_session)


def test_batch_bound_converges_on_repeated_runs(
    gc_session: Session, gc_storage: LocalFilesystemBlobStorage
) -> None:
    _seed_user(gc_session)
    orphans = [
        _put(gc_session, gc_storage, bytes([index]) * 4, created_at=NOW - timedelta(days=7))
        for index in range(5)
    ]
    gc_session.commit()

    first = _run(gc_session, gc_storage, batch_size=2)
    gc_session.commit()
    second = _run(gc_session, gc_storage, batch_size=2)
    gc_session.commit()
    third = _run(gc_session, gc_storage, batch_size=2)
    gc_session.commit()

    assert first.deleted_count == 2
    assert second.deleted_count == 2
    assert third.deleted_count == 1
    for orphan in orphans:
        assert orphan not in _registered_hashes(gc_session)


def test_gc_is_idempotent(
    gc_session: Session, gc_storage: LocalFilesystemBlobStorage
) -> None:
    _seed_user(gc_session)
    _put(gc_session, gc_storage, b"orphan-once", created_at=NOW - timedelta(days=7))
    gc_session.commit()

    first = _run(gc_session, gc_storage)
    gc_session.commit()
    second = _run(gc_session, gc_storage)
    gc_session.commit()

    assert first.deleted_count == 1
    assert second.deleted_count == 0
