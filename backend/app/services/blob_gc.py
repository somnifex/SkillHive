"""Server blob garbage collection (M2.2 destructive sweep, plan §9.7).

Implements the mark-and-sweep contract from ``docs/development/GC_DESIGN.md``:

- **Mark** roots are recomputed from durable relational state on every run
  (never a mutable refcount): every retained version manifest hash, every
  Skill's current package hash, change-log rows inside the retention window
  (R2), unexpired mutation receipts referencing package data (R3), injected
  legal holds (R5), and — via a creation-time cutoff — in-flight uploads
  (R4). Each rooted manifest expands to its full package closure so a swept
  manifest can never orphan its file blobs (or vice versa).
- **Sweep** deletes registry rows whose objects are no longer marked, bytes
  first. The registry is the sweep basis, not a filesystem walk; a crash at
  any point is safe in both directions: a row whose bytes are gone makes
  negotiation report the object missing (the client re-uploads, verified and
  idempotent), and orphaned bytes without a row are invisible to negotiation
  and collected by a later sweep's filesystem reconciliation.

The job is idempotent and bounded per invocation; repeated runs converge.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timedelta

from sqlalchemy import select
from sqlalchemy.orm import Session

from app.models import Skill, SkillBlobObject, SkillVersion, SyncChangeLog, SyncMutationReceipt
from app.services.blob_storage import BlobStorage
from app.services.package_manifest import validate_snapshot_manifest_bytes


@dataclass
class GcReport:
    """One run's outcome, safe to log (hashes only, never blob contents)."""

    root_count: int = 0
    marked_count: int = 0
    candidate_count: int = 0
    deleted_count: int = 0
    deleted_hashes: list[str] = field(default_factory=list)


def run_blob_gc(
    session: Session,
    storage: BlobStorage,
    *,
    now: datetime,
    orphan_grace: timedelta,
    change_retention: timedelta,
    receipt_retention: timedelta,
    batch_size: int = 1000,
    legal_roots: frozenset[str] | set[str] = frozenset(),
) -> GcReport:
    """Run one bounded mark-and-sweep pass. Safe to re-run at any point.

    The caller owns the transaction (commit after the report returns). The
    mark set is computed from the caller's snapshot; the sweep cutoff keeps
    anything created inside the orphan grace regardless of marks.
    """
    report = GcReport()
    mark_started = now

    # --- Mark phase -------------------------------------------------------
    roots: set[str] = set(legal_roots)  # R5: retention/legal policy pins
    roots.update(_all_version_manifest_hashes(session))  # R1
    roots.update(_current_package_hashes(session))  # R1 (explicit root)
    roots.update(
        _change_log_manifest_hashes(session, now - change_retention)  # R2
    )
    roots.update(
        _receipt_payload_manifest_hashes(session, now - receipt_retention)  # R3
    )
    report.root_count = len(roots)

    # Closure expansion: every rooted manifest pulls in its file blobs. An
    # unreadable manifest (bytes lost mid-crash, corrupt historical row)
    # contributes no expansion — its own hash stays marked, so the sweep
    # never deletes a rooted manifest's row.
    marked: set[str] = set()
    frontier = list(roots)
    while frontier:
        manifest_hash = frontier.pop()
        if manifest_hash in marked:
            continue
        marked.add(manifest_hash)
        for blob_hash in _manifest_closure(session, storage, manifest_hash):
            if blob_hash not in marked:
                frontier.append(blob_hash)
    report.marked_count = len(marked)

    # --- Sweep phase ------------------------------------------------------
    # R4 (in-flight uploads) is enforced by the created_at cutoff: anything
    # registered inside the orphan grace is kept regardless of marks.
    grace_cutoff = mark_started - orphan_grace
    candidates = list(
        session.scalars(
            select(SkillBlobObject)
            .where(SkillBlobObject.created_at < grace_cutoff)
            .order_by(SkillBlobObject.hash.asc())
            .limit(batch_size)
        )
    )
    report.candidate_count = len(candidates)

    for obj in candidates:
        if obj.hash in marked:
            continue
        # Bytes first, then the row: a row without bytes makes negotiation
        # report the object missing (client re-uploads, idempotent); bytes
        # without a row are invisible to negotiation and collected by the
        # next sweep. ``storage.delete`` tolerates absent objects.
        storage.delete(obj.hash)
        session.delete(obj)
        report.deleted_count += 1
        report.deleted_hashes.append(obj.hash)

    return report


def _all_version_manifest_hashes(session: Session) -> set[str]:
    return set(
        session.scalars(
            select(SkillVersion.package_manifest_hash).where(
                SkillVersion.package_manifest_hash.is_not(None)
            )
        )
    )


def _current_package_hashes(session: Session) -> set[str]:
    return set(
        session.scalars(
            select(Skill.current_package_hash).where(Skill.current_package_hash.is_not(None))
        )
    )


def _change_log_manifest_hashes(session: Session, since: datetime) -> set[str]:
    rows = session.scalars(
        select(SyncChangeLog.package_manifest_hash).where(
            SyncChangeLog.package_manifest_hash.is_not(None),
            SyncChangeLog.created_at >= since,
        )
    )
    return {row for row in rows if row is not None}


def _receipt_payload_manifest_hashes(session: Session, since: datetime) -> set[str]:
    """Manifest hashes referenced by unexpired receipts (R3, defense-in-depth).

    Replay is served from the persisted payload JSON, so this root only
    matters for clients that re-fetch the manifest after a replay. Payload
    JSON is parsed defensively: a malformed historical payload must never
    abort the sweep.
    """
    hashes: set[str] = set()
    receipts = session.scalars(
        select(SyncMutationReceipt).where(SyncMutationReceipt.created_at >= since)
    )
    for receipt in receipts:
        payload = receipt.response_payload or {}
        result = payload.get("result") if isinstance(payload, dict) else None
        manifest_hash = result.get("packageManifestHash") if isinstance(result, dict) else None
        if isinstance(manifest_hash, str):
            hashes.add(manifest_hash)
    return hashes


def _manifest_closure(session: Session, storage: BlobStorage, manifest_hash: str) -> set[str]:
    """Expand one manifest hash to the file blob hashes it references.

    Malformed or unreadable manifests yield an empty expansion: the sweep
    must survive any historical debris, and keeping extra objects is always
    safe while deleting referenced ones is not.
    """
    del session  # the registry check is unnecessary: storage.open verifies
    try:
        with storage.open(manifest_hash) as stream:
            manifest_bytes = stream.read()
        files = validate_snapshot_manifest_bytes(manifest_bytes)
    except Exception:  # noqa: BLE001 - sweep must survive any bad manifest
        return set()
    return {str(entry["blob_hash"]) for entry in files}


def parse_manifest_closure(manifest_bytes: bytes) -> set[str]:
    """Parse manifest bytes and return the referenced blob hashes.

    Shared with tests; invalid manifests yield an empty set.
    """
    try:
        files = validate_snapshot_manifest_bytes(manifest_bytes)
    except Exception:  # noqa: BLE001 - sweep must survive any bad manifest
        return set()
    return {str(entry["blob_hash"]) for entry in files}
