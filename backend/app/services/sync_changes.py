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

from sqlalchemy import select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.models import GroupMember, GroupSkillGrant, Skill, SyncChangeLog
from app.schemas.sync import SyncChangeItem, SyncChangesResponse
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
    last_sequence = page[-1].sequence if page else sequence
    return SyncChangesResponse(
        protocol_version=1,
        changes=changes,
        next_cursor=encode_sync_cursor(last_sequence),
        has_more=has_more,
        server_time=datetime.now(UTC),
    )


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

    return SyncChangeItem(
        sequence=row.sequence,
        resource_type="skill",
        resource_id=row.resource_id,
        resource_revision=row.resource_revision,
        operation=row.operation,
        package_manifest_hash=row.package_manifest_hash,
        metadata=metadata,
        created_at=row.created_at,
    )


def _skill_visible(
    session: Session, skill: Skill | None, row: SyncChangeLog, user_id: str
) -> bool:
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
