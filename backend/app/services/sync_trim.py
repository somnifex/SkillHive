"""Server change-log/receipt trim jobs (M4, GC_DESIGN.md §6 + §10 item 1).

The destructive blob sweep (``blob_gc.py``) landed with the retention
settings already configured; this module completes the same work package:
bounded deletion of expired ``sync_change_log`` and
``sync_mutation_receipts`` rows so the pull feed and receipt table stop
growing without bound.

Contract highlights (GC design §6):

- **Change-log trim** deletes rows older than ``change_log_retention_days``.
  The ``SYNC_CURSOR_EXPIRED`` (410) check in ``sync_changes.list_changes``
  is already live and keys off the oldest surviving row, so a client whose
  cursor predates the trimmed history fails loudly instead of receiving a
  silently truncated page.
- **Receipt trim** deletes rows older than ``receipt_retention_days``.
  Receipts are the idempotency basis for push; a trimmed receipt means an
  extremely late replay re-applies. That is acceptable because the desktop
  retires `acked` mutations locally and never replays past the retention
  horizon (GC_DESIGN.md §6).
- Both jobs are **bounded per invocation** (``batch_size``) so a huge backlog
  cannot pin the database; repeated runs converge.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timedelta

from sqlalchemy import delete, select
from sqlalchemy.orm import Session

from app.models import SyncChangeLog, SyncMutationReceipt


@dataclass
class TrimReport:
    """One run's outcome; safe to log (counts and sequence bounds only)."""

    change_log_deleted: int = 0
    receipts_deleted: int = 0
    oldest_retained_sequence: int | None = None


@dataclass
class _DeleteBatch:
    """A batch of primary keys selected for deletion."""

    change_sequences: list[int] = field(default_factory=list)
    receipt_ids: list[str] = field(default_factory=list)


def trim_expired_rows(
    session: Session,
    *,
    now: datetime,
    change_retention: timedelta,
    receipt_retention: timedelta,
    batch_size: int = 1000,
) -> TrimReport:
    """Delete expired change-log and receipt rows, bounded per invocation.

    The caller owns the transaction (commit after the report returns).
    Selection is ordered and key-bounded so repeated runs converge without
    ever holding a full-table delete open.
    """
    report = TrimReport()

    change_cutoff = now - change_retention
    expired_sequences = list(
        session.scalars(
            select(SyncChangeLog.sequence)
            .where(SyncChangeLog.created_at < change_cutoff)
            .order_by(SyncChangeLog.sequence.asc())
            .limit(batch_size)
        )
    )
    if expired_sequences:
        session.execute(
            delete(SyncChangeLog).where(SyncChangeLog.sequence.in_(expired_sequences))
        )
        report.change_log_deleted = len(expired_sequences)

    receipt_cutoff = now - receipt_retention
    expired_receipt_ids = list(
        session.scalars(
            select(SyncMutationReceipt.id)
            .where(SyncMutationReceipt.created_at < receipt_cutoff)
            .order_by(SyncMutationReceipt.created_at.asc())
            .limit(batch_size)
        )
    )
    if expired_receipt_ids:
        session.execute(
            delete(SyncMutationReceipt).where(SyncMutationReceipt.id.in_(expired_receipt_ids))
        )
        report.receipts_deleted = len(expired_receipt_ids)

    oldest = session.scalar(
        select(SyncChangeLog.sequence).order_by(SyncChangeLog.sequence.asc()).limit(1)
    )
    report.oldest_retained_sequence = int(oldest) if oldest is not None else None
    return report
