from __future__ import annotations

from types import SimpleNamespace

import pytest
from app.services.blob_gc import GcReport
from app.services.sync_trim import TrimReport


def test_maintenance_pass_serializes_bounded_jobs_and_commits(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import app.main as main

    events: list[str] = []

    class FakeSession:
        def __enter__(self) -> FakeSession:
            events.append("open")
            return self

        def __exit__(self, *_args: object) -> None:
            events.append("close")

        def commit(self) -> None:
            events.append("commit")

    sessions = [FakeSession(), FakeSession(), FakeSession()]
    monkeypatch.setattr(main, "SessionLocal", lambda: sessions.pop(0))
    def purge(_session: object) -> int:
        events.append("trash")
        return 2

    def trim(*_args: object, **_kwargs: object) -> TrimReport:
        events.append("trim")
        return TrimReport(
            change_log_deleted=3,
            receipts_deleted=4,
        )

    def gc(*_args: object, **_kwargs: object) -> GcReport:
        events.append("gc")
        return GcReport(candidate_count=5, deleted_count=1)

    monkeypatch.setattr(main, "purge_expired_trash", purge)
    monkeypatch.setattr(main, "trim_expired_rows", trim)
    monkeypatch.setattr(main, "get_blob_storage", lambda _session: object())
    monkeypatch.setattr(main, "run_blob_gc", gc)
    monkeypatch.setattr(
        main,
        "settings",
        SimpleNamespace(
            change_log_retention_days=90,
            receipt_retention_days=90,
            blob_gc_orphan_grace_hours=24,
            blob_gc_batch_size=7,
        ),
    )

    main._maintenance_once()

    assert events == [
        "open",
        "trash",
        "close",
        "open",
        "trim",
        "commit",
        "close",
        "open",
        "gc",
        "commit",
        "close",
    ]
