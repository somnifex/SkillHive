"""Automatic purge of expired trash entries (admin-configurable retention).

Skills soft-deleted into the trash are purged permanently once their
``deleted_at`` exceeds ``trash_retention_days`` (system setting, default 30).
A retention of ``0`` disables automatic purging entirely — the trash only
shrinks when a user or admin purges explicitly.

The sweep is deliberately conservative: one failing row must never abort the
whole run, and failures are logged (never raised) so a background hiccup can
never take the API down.
"""

from __future__ import annotations

from datetime import timedelta

from sqlalchemy import select
from sqlalchemy.orm import Session

from app.db.base import utc_now
from app.models import Skill
from app.services.skill_mutations import SkillMutationService
from app.services.system_settings import trash_retention_days

AUDIT_ACTION = "skill.trash_autopurged"


def expired_trash_ids(session: Session) -> list[str]:
    """Resolves the ids of trashed skills past the retention deadline.

    Returns ids (not ORM rows) so each purge below can run inside its own
    nested transaction without holding a stale row across commits.
    """
    retention = trash_retention_days(session)
    if retention <= 0:
        return []
    deadline = utc_now() - timedelta(days=retention)
    return list(
        session.scalars(
            select(Skill.id).where(
                Skill.status == "deleted",
                Skill.deleted_at.is_not(None),
                Skill.deleted_at < deadline,
            )
        )
    )


def purge_expired_trash(session: Session) -> int:
    """Purges every trashed skill whose retention window has elapsed.

    Returns the number of purged skills. Entries are purged one savepoint at
    a time so a single failure (e.g. a dangling grant edge) skips that row
    instead of poisoning the sweep; audit rows and tombstones ride the same
    transaction as the delete via the shared mutation service.
    """
    ids = expired_trash_ids(session)
    purged = 0
    for skill_id in ids:
        nested = session.begin_nested()
        try:
            skill = session.get(Skill, skill_id)
            if skill is None or skill.status != "deleted":
                nested.rollback()
                continue
            SkillMutationService(session, actor_user_id=skill.created_by or "system").purge_skill(
                skill, audit_action=AUDIT_ACTION
            )
            nested.commit()
            purged += 1
        except Exception:  # noqa: BLE001 - one bad row must not stop the sweep
            nested.rollback()
    if ids:
        session.commit()
    return purged
