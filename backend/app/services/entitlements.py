"""M3 permission/entitlement leases for managed skills (handoff §15.1).

A lease is the server-signed statement a desktop keeps with cached managed
content. It binds one (user, group grant, skill) to a permission level and
an offline deadline so the client can enforce the TTL policy locally while
the server stays authoritative: the pull projection ships the current
lease for every visible managed skill, and the mutation path verifies a
still-valid lease before accepting managed-skill writes.

Leases are signed with the existing JWT secret (``settings.jwt_secret_key``)
using the standard ``create_token``/``decode_token`` machinery — same
signing key, different ``type`` claim, so an access token can never be
replayed as a lease and vice versa.

Policy semantics (roadmap M3):

- ``unlimited``: offline use never expires; the lease still carries an
  ``exp`` claim (server clock + ``LEASE_UNLIMITED_TTL_HOURS``) so a stale
  lease must be refreshed from the server rather than cached forever.
- ``ttl``: offline use expires ``offline_ttl_hours`` after the lease is
  issued.
- ``disabled``: no offline window — the lease expires immediately
  (zero-ttl) and only live, server-connected access remains.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from typing import Any

import jwt
from jwt import InvalidTokenError

from app.core.config import settings
from app.core.exceptions import AppError
from app.models import GroupSkillGrant, Skill

LEASE_TOKEN_TYPE = "skill_lease"
#: Refresh cadence for ``unlimited`` leases: the client must re-verify the
#: entitlement with the server this often even though access never expires.
LEASE_UNLIMITED_TTL_HOURS = 24 * 7


@dataclass(frozen=True)
class EntitlementLease:
    """The client-visible lease payload for one managed skill."""

    skill_id: str
    permission_level: str
    offline_policy: str
    offline_ttl_hours: int | None
    issued_at: datetime
    expires_at: datetime
    policy_version: int

    def as_claims(self) -> dict[str, Any]:
        return {
            "skill_id": self.skill_id,
            "permission": self.permission_level,
            "offline_policy": self.offline_policy,
            "offline_ttl_hours": self.offline_ttl_hours,
            "policy_version": self.policy_version,
            "type": LEASE_TOKEN_TYPE,
            "iat": self.issued_at,
            "exp": self.expires_at,
        }


def _policy_version(grant: GroupSkillGrant) -> int:
    """Derives a monotone policy version from the grant's mutable fields.

    ``updated_at`` is bumped by TimestampMixin on every grant update, so
    its epoch seconds are a stable, collision-resistant version for a
    single grant row.
    """
    updated = grant.updated_at or grant.created_at
    if updated.tzinfo is None:
        updated = updated.replace(tzinfo=UTC)
    return int(updated.timestamp())


def lease_expiry(grant: GroupSkillGrant, *, now: datetime) -> datetime:
    """Expiry per the grant's offline policy (handoff §15.1 policy table)."""
    if grant.offline_policy == "ttl":
        ttl = int(grant.offline_ttl_hours or 0)
        if ttl < 1:
            raise AppError(
                "OFFLINE_POLICY_INVALID",
                "Grant offline TTL policy requires a positive hour bound.",
                500,
            )
        return now + timedelta(hours=ttl)
    if grant.offline_policy == "disabled":
        # No offline window: the lease is dead on arrival. ``exp == now``
        # keeps the signed shape uniform while making offline use impossible.
        return now
    return now + timedelta(hours=LEASE_UNLIMITED_TTL_HOURS)


def issue_entitlement_lease(
    grant: GroupSkillGrant,
    skill: Skill,
    *,
    now: datetime | None = None,
) -> tuple[str, EntitlementLease]:
    """Signs and returns the lease token and its decoded claims."""
    issued_at = now or datetime.now(UTC)
    if issued_at.tzinfo is None:
        issued_at = issued_at.replace(tzinfo=UTC)
    expires_at = lease_expiry(grant, now=issued_at)
    lease = EntitlementLease(
        skill_id=skill.id,
        permission_level="use",
        offline_policy=grant.offline_policy,
        offline_ttl_hours=grant.offline_ttl_hours,
        issued_at=issued_at,
        expires_at=expires_at,
        policy_version=_policy_version(grant),
    )
    token = jwt.encode(
        lease.as_claims(),
        settings.jwt_secret_key,
        algorithm=settings.jwt_algorithm,
    )
    return token, lease


def decode_entitlement_lease(token: str) -> dict[str, Any]:
    """Decodes and structurally validates a lease token (server-side)."""
    try:
        payload: dict[str, Any] = jwt.decode(
            token,
            settings.jwt_secret_key,
            algorithms=[settings.jwt_algorithm],
        )
    except InvalidTokenError as error:
        raise AppError("LEASE_INVALID", "Entitlement lease is invalid or expired.", 401) from error
    if payload.get("type") != LEASE_TOKEN_TYPE:
        raise AppError("LEASE_INVALID", "Token is not an entitlement lease.", 401)
    for claim in ("skill_id", "permission", "offline_policy", "policy_version"):
        if not payload.get(claim) and payload.get(claim) != 0:
            raise AppError("LEASE_INVALID", f"Lease is missing the {claim!r} claim.", 401)
    return payload
