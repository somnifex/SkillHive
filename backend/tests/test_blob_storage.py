"""M2.2 blob storage backend and registry tests.

Covers the storage invariants from the local validation checklist PART K §36:
content addressing, digest mismatch rejection, duplicate upload safety,
symlink rejection, and registry metadata/bytes consistency.
"""

from collections.abc import Generator
from pathlib import Path

import pytest
from app.db.base import Base
from app.models import SkillBlobObject
from app.schemas.sync import SyncBlobDescriptor
from app.services.blob_registry import missing_blobs, register_verified_blob
from app.services.blob_storage import (
    BlobStorageError,
    LocalFilesystemBlobStorage,
    hash_bytes,
)
from sqlalchemy import create_engine
from sqlalchemy.orm import Session, sessionmaker


@pytest.fixture
def session(tmp_path: Path) -> Generator[Session, None, None]:
    engine = create_engine(
        f"sqlite:///{(tmp_path / 'test.db').as_posix()}",
        connect_args={"check_same_thread": False},
    )
    Base.metadata.create_all(engine)
    factory = sessionmaker(bind=engine, autoflush=False, expire_on_commit=False)
    with factory() as instance:
        yield instance
    engine.dispose()


def test_put_is_content_addressed_and_idempotent(tmp_path: Path) -> None:
    storage = LocalFilesystemBlobStorage(tmp_path / "blobs")
    payload = b"hello SkillHive"
    hash_value = hash_bytes(payload)

    storage.put_verified(hash_value, payload)
    storage.put_verified(hash_value, payload)  # duplicate write must not corrupt

    assert storage.exists(hash_value, len(payload))
    with storage.open(hash_value) as stream:
        assert stream.read() == payload


def test_put_rejects_digest_mismatch(tmp_path: Path) -> None:
    storage = LocalFilesystemBlobStorage(tmp_path / "blobs")
    wrong_hash = hash_bytes(b"other bytes")

    with pytest.raises(BlobStorageError):
        storage.put_verified(wrong_hash, b"different bytes")

    assert not (tmp_path / "blobs" / "sha256").exists() or not storage.exists(
        wrong_hash, len(b"different")
    )


def test_detects_corruption_on_read(tmp_path: Path) -> None:
    storage = LocalFilesystemBlobStorage(tmp_path / "blobs")
    payload = b"original"
    hash_value = hash_bytes(payload)
    storage.put_verified(hash_value, payload)

    digest = hash_value.removeprefix("sha256:")
    blob_path = tmp_path / "blobs" / "sha256" / digest[:2] / digest
    blob_path.write_bytes(b"corrupted")

    assert not storage.exists(hash_value, len(payload))


def test_rejects_symlink_entry(tmp_path: Path) -> None:
    root = tmp_path / "blobs"
    storage = LocalFilesystemBlobStorage(root)
    payload = b"symlink target"
    hash_value = hash_bytes(payload)
    storage.put_verified(hash_value, payload)

    digest = hash_value.removeprefix("sha256:")
    blob_path = root / "sha256" / digest[:2] / digest

    try:
        blob_path.unlink()
        blob_path.symlink_to(root / "sha256" / "outside-target")
    except OSError:
        # Windows without symlink privilege: cannot construct the scenario.
        return

    with pytest.raises(BlobStorageError):
        storage.open(hash_value)


def test_missing_blobs_round_trip(tmp_path: Path, session: Session) -> None:
    storage = LocalFilesystemBlobStorage(tmp_path / "blobs")
    stored_payload = b"stored"
    stored_hash = hash_bytes(stored_payload)
    absent_payload = b"absent"
    absent_hash = hash_bytes(absent_payload)

    register_verified_blob(session, storage, stored_hash, stored_payload)

    missing = missing_blobs(
        session,
        storage,
        [
            SyncBlobDescriptor(hash=stored_hash, size_bytes=len(stored_payload)),
            SyncBlobDescriptor(hash=absent_hash, size_bytes=len(absent_payload)),
        ],
    )
    assert [item.hash for item in missing] == [absent_hash]
    assert stored_hash != absent_hash


def test_row_without_bytes_reported_missing(tmp_path: Path, session: Session) -> None:
    storage = LocalFilesystemBlobStorage(tmp_path / "blobs")
    payload = b"package bytes"
    hash_value = hash_bytes(payload)

    # Simulate metadata/bytes divergence: a metadata row exists but the
    # physical object was never stored (e.g. lost during backend migration).
    session.add(
        SkillBlobObject(
            hash=hash_value,
            size_bytes=len(payload),
            storage_key=hash_value,
            storage_backend="local_filesystem",
        )
    )
    session.flush()

    missing = missing_blobs(
        session,
        storage,
        [SyncBlobDescriptor(hash=hash_value, size_bytes=len(payload))],
    )
    assert len(missing) == 1


def test_invalid_hash_rejected(tmp_path: Path) -> None:
    storage = LocalFilesystemBlobStorage(tmp_path / "blobs")
    with pytest.raises(BlobStorageError):
        storage.put_verified("not-a-hash", b"bytes")
