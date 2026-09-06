"""M4 change-log/receipt trim tests (GC_DESIGN.md §6 + §10 item 1).

Covers the bounded trim contract over real SQLite: rows inside the
retention window survive, expired rows are deleted, the SYNC_CURSOR_EXPIRED
contract keys off the trimmed oldest sequence, both jobs are bounded per
run (repeated runs converge), and nothing within retention is ever touched.
"""

from __future__ import annotations

from collections.abc import Generator
from datetime import UTC, datetime, timedelta
from typing import Any
from uuid import uuid4

import pytest
from app.db.base import Base
from app.models import SyncChangeLog, SyncMutationReceipt
from app.services.sync_trim import trim_expired_rows
from sqlalchemy import create_engine, func, select
from sqlalchemy.orm import Session, sessionmaker

NOW = datetime(2026, 9, 6, 12, 0, 0, tzinfo=UTC)


@pytest.fixture
def trim_session(tmp_path: object) -> Generator[Session, None, None]:
    engine = create_engine(
        f"sqlite:///{tmp_path / 'trim.db'}",  # type: ignore[operator]
        connect_args={"check_same_thread": False},
    )
    Base.metadata.create_all(engine)
    factory = sessionmaker(bind=engine, autoflush=False, expire_on_commit=False)
    session = factory()
    yield session
    session.close()
    engine.dispose()


def _change_row(session: Session, *, created_at: datetime, sequence: int | None = None) -> int:
    row = SyncChangeLog(
        resource_type="skill",
        resource_id=uuid4().hex,
        resource_revision=1,
        operation="upsert",
        owner_user_id=None,
        package_manifest_hash=None,
        metadata_payload={},
        created_at=created_at,
    )
    if sequence is not None:
        row.sequence = sequence
    session.add(row)
    session.flush()
    return int(row.sequence)


def _receipt_row(session: Session, *, created_at: datetime) -> str:
    row = SyncMutationReceipt(
        user_id=uuid4().hex,
        device_id=uuid4().hex,
        mutation_id=uuid4().hex,
        operation="create",
        resource_type="skill",
        resource_id=None,
        result_code="acked",
        result_revision=1,
        response_payload={},
        created_at=created_at,
    )
    session.add(row)
    session.flush()
    return row.id


def _run(session: Session, **overrides: Any) -> Any:
    kwargs: dict[str, Any] = {
        "now": NOW,
        "change_retention": timedelta(days=90),
        "receipt_retention": timedelta(days=90),
    }
    kwargs.update(overrides)
    return trim_expired_rows(session, **kwargs)


def _change_count(session: Session) -> int:
    return int(session.scalar(select(func.count()).select_from(SyncChangeLog)))


def _receipt_count(session: Session) -> int:
    return int(session.scalar(select(func.count()).select_from(SyncMutationReceipt)))


def test_recent_rows_survive(trim_session: Session) -> None:
    _change_row(trim_session, created_at=NOW - timedelta(days=7))
    _receipt_row(trim_session, created_at=NOW - timedelta(days=7))
    trim_session.commit()

    report = _run(trim_session)
    trim_session.commit()

    assert report.change_log_deleted == 0
    assert report.receipts_deleted == 0
    assert _change_count(trim_session) == 1
    assert _receipt_count(trim_session) == 1


def test_expired_rows_are_deleted(trim_session: Session) -> None:
    recent_sequence = _change_row(trim_session, created_at=NOW - timedelta(days=7))
    stale_sequence = _change_row(trim_session, created_at=NOW - timedelta(days=180))
    _receipt_row(trim_session, created_at=NOW - timedelta(days=7))
    _receipt_row(trim_session, created_at=NOW - timedelta(days=180))
    trim_session.commit()

    report = _run(trim_session)
    trim_session.commit()

    assert report.change_log_deleted == 1
    assert report.receipts_deleted == 1
    assert _change_count(trim_session) == 1
    assert _receipt_count(trim_session) == 1
    surviving = int(trim_session.scalar(select(SyncChangeLog.sequence).limit(1)))
    assert surviving == recent_sequence
    assert stale_sequence not in (recent_sequence, surviving)


def test_trim_updates_oldest_retained_sequence_for_cursor_contract(
    trim_session: Session,
) -> None:
    """After trimming, the oldest surviving sequence must be servable.

    ``sync_changes.list_changes`` fails with SYNC_CURSOR_EXPIRED when a
    cursor points below the oldest retained row; the trim job's report
    exposes exactly that bound so operators can see when clients will
    re-baseline.
    """
    stale = _change_row(trim_session, created_at=NOW - timedelta(days=200))
    _change_row(trim_session, created_at=NOW - timedelta(days=7))
    trim_session.commit()
    assert stale is not None

    report = _run(trim_session)
    trim_session.commit()

    assert report.oldest_retained_sequence is not None
    assert report.oldest_retained_sequence > stale


def test_batch_bound_converges_on_repeated_runs(trim_session: Session) -> None:
    for days in (100, 120, 140, 160, 180, 200):
        _change_row(trim_session, created_at=NOW - timedelta(days=days))
        _receipt_row(trim_session, created_at=NOW - timedelta(days=days))
    trim_session.commit()

    first = _run(trim_session, batch_size=3)
    assert first.change_log_deleted == 3
    assert first.receipts_deleted == 3

    second = _run(trim_session, batch_size=3)
    trim_session.commit()
    assert second.change_log_deleted == 3
    assert second.receipts_deleted == 3
    assert _change_count(trim_session) == 0
    assert _receipt_count(trim_session) == 0

    third = _run(trim_session)
    assert third.change_log_deleted == 0
    assert third.receipts_deleted == 0
    assert third.oldest_retained_sequence is None


def test_trim_is_noop_on_empty_tables(trim_session: Session) -> None:
    report = _run(trim_session)
    assert report.change_log_deleted == 0
    assert report.receipts_deleted == 0
    assert report.oldest_retained_sequence is None
