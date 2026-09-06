"""M2.5 durable pull feed projection.

Projects the append-only ``sync_change_log`` into the pull protocol with
current server-authorization filtering (plan §12):

- private Skills: visible to the owner only, tombstones included;
- global Skills: visible while published; members with an active grant keep
  receiving the delete tombstone after deletion;
- group Skills: visible when the Skill is global+published and the caller has
  an active group membership with an active grant.

Private bodies are never exposed to a global administrator through pull; the
existing ACL semantics decide visibility. A committed change-log row the
caller may not currently see is skipped; its sequence is still consumed by the
cursor advance, so a later permission grant re-ships the resource as a new
event.
"""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from sqlalchemy import func, select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.models import GroupMember, GroupSkillGrant, Skill, SkillVersion, SyncChangeLog
from app.schemas.sync import SyncChangeItem, SyncChangesResponse
from app.services.blob_storage import get_blob_storage
from app.services.entitlements import issue_entitlement_lease
from app.services.legacy_package import synthesize_legacy_package
from app.services.sync_cursor import SyncCursorError, decode_sync_cursor, encode_sync_cursor

DEFAULT_PULL_LIMIT = 100
MAX_PULL_LIMIT = 200

_RESOURCE_SKILL = "skill"
_OPERATION_DELETE = "delete"


def list_changes(
    session: Session,
    *,
    user_id: str,
    cursor: str | None,
    limit: int,
) -> SyncChangesResponse:
    """Return one bounded, stably ordered page of visible change entries."""
    try:
        sequence = decode_sync_cursor(cursor)
    except SyncCursorError as error:
        raise AppError("SYNC_CURSOR_INVALID", "Malformed sync cursor.", 400) from error
    bounded_limit = min(max(1, limit), MAX_PULL_LIMIT)

    # Cursor-expiry contract (GC design §6): a cursor pointing before the
    # oldest retained change row must fail loudly, never serve a silently
    # truncated page. Sequence 0 (no cursor) means full baseline and is
    # always servable — the baseline is rebuilt from live state, not history.
    if sequence > 0:
        oldest = _oldest_retained_sequence(session)
        if oldest is not None and sequence < oldest:
            raise AppError(
                "SYNC_CURSOR_EXPIRED",
                "Cursor predates retained history; re-baseline required.",
                410,
            )

    statement = (
        select(SyncChangeLog)
        .where(SyncChangeLog.sequence > sequence)
        .order_by(SyncChangeLog.sequence.asc())
        .limit(bounded_limit + 1)
    )
    rows = list(session.scalars(statement))
    has_more = len(rows) > bounded_limit
    page = rows[:bounded_limit]

    changes = [
        item for item in (_project(session, row, user_id) for row in page) if item is not None
    ]
    # Lazy legacy synthesis (plan §17): a pre-existing Skill with no package
    # gets one the first time it is pulled. The synthesized blobs are
    # verified and the Skill row updated before the page is returned, so the
    # shipped hash is immediately addressable via the blob endpoint.
    if session.new or session.dirty:
        session.commit()
    last_sequence = page[-1].sequence if page else sequence
    return SyncChangesResponse(
        protocol_version=1,
        changes=changes,
        next_cursor=encode_sync_cursor(last_sequence),
        has_more=has_more,
        server_time=datetime.now(UTC),
    )


def _oldest_retained_sequence(session: Session) -> int | None:
    """The oldest sequence still in the change log, or None when empty.

    With trimming (the future GC companion) rows older than the retention
    window are deleted; a cursor below the oldest surviving row can no
    longer be served contiguously and must fail with SYNC_CURSOR_EXPIRED.
    """
    oldest = session.scalar(func.min(SyncChangeLog.sequence))
    return int(oldest) if oldest is not None else None


def _project(session: Session, row: SyncChangeLog, user_id: str) -> SyncChangeItem | None:
    """Project one change row if the caller may currently see the resource."""
    if row.resource_type != _RESOURCE_SKILL:
        return None
    skill = session.get(Skill, row.resource_id)

    if not _skill_visible(session, skill, row, user_id):
        return None

    metadata = dict(row.metadata_payload or {})
    if skill is not None and row.operation != _OPERATION_DELETE:
        metadata.setdefault("name", skill.name)
        metadata.setdefault("slug", skill.slug)
        metadata.setdefault("skill_type", skill.skill_type)
        metadata.setdefault("status", skill.status)
        _ensure_legacy_package(session, skill)
        _attach_entitlement_lease(session, skill, user_id, metadata)

    # The Skill's current package identity is authoritative: the change-log
    # row may predate package synthesis (legacy rows) or the resource may
    # have advanced since the event fired.
    package_hash = (
        skill.current_package_hash
        if skill is not None and row.operation != _OPERATION_DELETE
        else row.package_manifest_hash
    )

    return SyncChangeItem(
        sequence=row.sequence,
        resource_type="skill",
        resource_id=row.resource_id,
        resource_revision=row.resource_revision,
        operation=row.operation,
        package_manifest_hash=package_hash,
        metadata=metadata,
        created_at=row.created_at,
    )


def _ensure_legacy_package(session: Session, skill: Skill) -> None:
    """Synthesize the minimal package for a legacy Skill exactly once.

    Idempotent: a Skill that already carries ``current_package_hash`` is
    untouched; synthesis itself is deterministic (same content → same
    hashes) so a racing repeated run converges on the same objects.
    """
    if skill.current_package_hash is not None:
        return
    current_version = (
        session.get(SkillVersion, skill.current_version_id)
        if skill.current_version_id is not None
        else None
    )
    if current_version is None:
        return
    manifest_hash = synthesize_legacy_package(
        session,
        get_blob_storage(),
        slug=skill.slug,
        name=skill.name,
        description=skill.description,
        content=dict(current_version.content or {}),
    )
    if manifest_hash is None:
        return
    skill.current_package_hash = manifest_hash
    current_version.package_manifest_hash = manifest_hash


def _attach_entitlement_lease(
    session: Session,
    skill: Skill,
    user_id: str,
    metadata: dict[str, Any],
) -> None:
    """Ships the current signed lease for a managed (grant-gated) skill.

    Only group-managed skills carry leases (M3): the caller's active grant
    decides both visibility and the offline policy. Personal skills have no
    lease — their offline use is the user's own content. Lease issuance is
    deterministic per (grant state, current time) and never fails the pull:
    a grant that vanished between the visibility check and here simply
    ships no lease.
    """
    if skill.skill_type != "global":
        return
    grant = session.scalar(
        select(GroupSkillGrant).where(
            GroupSkillGrant.skill_id == skill.id,
            GroupSkillGrant.status == "active",
            GroupSkillGrant.group_id.in_(
                select(GroupMember.group_id).where(
                    GroupMember.user_id == user_id,
                    GroupMember.status == "active",
                )
            ),
        )
    )
    if grant is None:
        return
    token, _lease = issue_entitlement_lease(grant, skill)
    metadata["entitlement"] = {
        "lease": token,
        "permission_level": "use",
        "offline_policy": grant.offline_policy,
        "offline_ttl_hours": grant.offline_ttl_hours,
    }


def _skill_visible(session: Session, skill: Skill | None, row: SyncChangeLog, user_id: str) -> bool:
    if row.operation == _OPERATION_DELETE:
        # Tombstones are deterministic: the owner always sees their private
        # Skill's deletion; grant-holding group members keep seeing global
        # deletions so caches can be purged deterministically.
        if skill is None:
            return False
        if skill.owner_user_id == user_id:
            return True
        return _has_active_grant(session, skill.id, user_id)

    if skill is None:
        return False

    if skill.skill_type == "private":
        # Private content: owner only. Global administrators do not receive
        # private bodies through pull (plan §12).
        return skill.owner_user_id == user_id

    if skill.skill_type == "global":
        # Draft/disabled global Skills stay invisible until republished.
        return skill.status == "published"

    return False


def _has_active_grant(session: Session, skill_id: str, user_id: str) -> bool:
    group_ids = list(
        session.scalars(
            select(GroupMember.group_id).where(
                GroupMember.user_id == user_id,
                GroupMember.status == "active",
            )
        )
    )
    if not group_ids:
        return False
    grant = session.scalar(
        select(GroupSkillGrant.id).where(
            GroupSkillGrant.group_id.in_(group_ids),
            GroupSkillGrant.skill_id == skill_id,
            GroupSkillGrant.status == "active",
        )
    )
    return grant is not None
