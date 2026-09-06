"""M3 entitlement lease service tests (handoff §15.1).

Covers lease issuance and signing semantics over real model rows: policy
expiry per the offline table (unlimited / ttl / disabled), token
round-tripping, cross-type rejection (an access token is never a lease),
tamper rejection, and policy-version monotonicity across grant updates.
"""

from collections.abc import Generator
from datetime import UTC, datetime, timedelta
from pathlib import Path

import jwt
import pytest
from app.core.config import settings
from app.core.exceptions import AppError
from app.core.security import create_token
from app.db.base import Base
from app.models import Group, GroupMember, GroupSkillGrant, Skill, User
from app.services.entitlements import (
    LEASE_TOKEN_TYPE,
    LEASE_UNLIMITED_TTL_HOURS,
    decode_entitlement_lease,
    issue_entitlement_lease,
    lease_expiry,
)
from sqlalchemy import create_engine
from sqlalchemy.orm import Session, sessionmaker


@pytest.fixture
def lease_session(tmp_path: Path) -> Generator[Session, None, None]:
    engine = create_engine(
        f"sqlite:///{(tmp_path / 'leases.db').as_posix()}",
        connect_args={"check_same_thread": False},
    )
    Base.metadata.create_all(engine)
    factory = sessionmaker(bind=engine, autoflush=False, expire_on_commit=False)
    session = factory()
    yield session
    session.close()
    engine.dispose()


def _seed_grant(session: Session, **grant_kwargs: object) -> tuple[GroupSkillGrant, Skill]:
    user = User(
        username="lease_admin",
        display_name="Lease Admin",
        email="lease_admin@example.com",
        password_hash="x",
        status="active",
        is_global_admin=True,
    )
    session.add(user)
    session.flush()
    group = Group(
        name="Lease Group",
        description="",
        group_type="platform",
        owner_id=user.id,
        created_by=user.id,
        status="active",
    )
    session.add(group)
    session.flush()
    session.add(GroupMember(group_id=group.id, user_id=user.id, role="owner", status="active"))
    skill = Skill(
        name="Managed Skill",
        slug="managed-skill",
        description="",
        skill_type="global",
        owner_user_id=None,
        category="",
        tags=[],
        status="published",
        sync_revision=1,
        created_by=user.id,
    )
    session.add(skill)
    session.flush()
    grant = GroupSkillGrant(
        group_id=group.id,
        skill_id=skill.id,
        version_policy="latest",
        status="active",
        granted_by=user.id,
        **grant_kwargs,
    )
    session.add(grant)
    session.flush()
    return grant, skill


def test_ttl_policy_uses_grant_hours(lease_session: Session) -> None:
    grant, skill = _seed_grant(
        lease_session, offline_policy="ttl", offline_ttl_hours=8
    )
    now = datetime(2026, 9, 6, 12, 0, tzinfo=UTC)
    assert lease_expiry(grant, now=now) == now + timedelta(hours=8)

    token, lease = issue_entitlement_lease(grant, skill, now=now)
    assert lease.expires_at == now + timedelta(hours=8)
    payload = decode_entitlement_lease(token)
    assert payload["skill_id"] == skill.id
    assert payload["offline_policy"] == "ttl"
    assert payload["offline_ttl_hours"] == 8
    assert payload["type"] == LEASE_TOKEN_TYPE


def test_unlimited_policy_gets_refresh_bound_not_infinity(lease_session: Session) -> None:
    grant, skill = _seed_grant(lease_session)
    now = datetime(2026, 9, 6, 12, 0, tzinfo=UTC)
    expires = lease_expiry(grant, now=now)
    assert expires == now + timedelta(hours=LEASE_UNLIMITED_TTL_HOURS)
    token, _lease = issue_entitlement_lease(grant, skill, now=now)
    payload = decode_entitlement_lease(token)
    assert payload["offline_policy"] == "unlimited"
    assert payload["offline_ttl_hours"] is None


def test_disabled_policy_lease_is_dead_on_arrival(lease_session: Session) -> None:
    grant, skill = _seed_grant(lease_session, offline_policy="disabled")
    now = datetime(2026, 9, 6, 12, 0, tzinfo=UTC)
    assert lease_expiry(grant, now=now) == now
    token, _lease = issue_entitlement_lease(grant, skill, now=now)
    # Issued at ``now`` with exp == now: already expired a moment later.
    # The raw JWT layer raises first; the service maps any invalid-token
    # condition to LEASE_INVALID.
    with pytest.raises(AppError) as exc:
        decode_entitlement_lease(token)
    assert exc.value.code == "LEASE_INVALID"


def test_expired_ttl_lease_rejected(lease_session: Session) -> None:
    grant, skill = _seed_grant(
        lease_session, offline_policy="ttl", offline_ttl_hours=1
    )
    # Anchor at the real wall clock: a fixed past timestamp would make the
    # token expire before the assertion runs.
    now = datetime.now(UTC)
    token, _lease = issue_entitlement_lease(grant, skill, now=now)

    # Immediately valid at issuance.
    payload = decode_entitlement_lease(token)
    assert payload["offline_ttl_hours"] == 1
    # A re-signed token whose exp has passed must decode as invalid: the exp
    # claim is what gates validity.
    expired_claims = dict(payload)
    expired_claims["exp"] = now - timedelta(hours=1)
    expired_token = jwt.encode(
        expired_claims,
        settings.jwt_secret_key,
        algorithm=settings.jwt_algorithm,
    )
    with pytest.raises(AppError) as exc:
        decode_entitlement_lease(expired_token)
    assert exc.value.code == "LEASE_INVALID"


def test_access_token_is_never_a_valid_lease(lease_session: Session) -> None:
    access, _jti, _exp = create_token("user-1", "access", timedelta(minutes=5))
    with pytest.raises(AppError) as exc:
        decode_entitlement_lease(access)
    assert exc.value.code == "LEASE_INVALID"


def test_tampered_lease_rejected(lease_session: Session) -> None:
    grant, skill = _seed_grant(lease_session)
    token, _lease = issue_entitlement_lease(grant, skill)
    header, body, signature = token.split(".")
    with pytest.raises(AppError) as exc:
        decode_entitlement_lease(f"{header}.{body}.AAAA{signature}")
    assert exc.value.code == "LEASE_INVALID"


def test_policy_version_advances_with_grant_update(lease_session: Session) -> None:
    grant, skill = _seed_grant(lease_session)
    # Anchor the grant's timestamps in the past so the explicit update below
    # is the newer state regardless of the real wall clock.
    anchor = datetime.now(UTC) - timedelta(days=2)
    grant.created_at = anchor
    grant.updated_at = anchor
    lease_session.flush()
    _token1, lease1 = issue_entitlement_lease(grant, skill)

    grant.offline_policy = "ttl"
    grant.offline_ttl_hours = 8
    grant.updated_at = anchor + timedelta(hours=1)
    lease_session.flush()

    _token2, lease2 = issue_entitlement_lease(grant, skill)
    assert lease2.policy_version > lease1.policy_version
    assert lease2.offline_policy == "ttl"
    assert lease2.offline_ttl_hours == 8


def test_ttl_without_hours_is_a_configuration_error(lease_session: Session) -> None:
    grant, _skill = _seed_grant(lease_session)
    # The schema forbids ttl-without-hours at the DB level, so simulate the
    # in-memory state the defensive branch protects against.
    grant.offline_policy = "ttl"
    lease_session.expire(grant)
    grant.offline_policy = "ttl"
    grant.offline_ttl_hours = None
    with pytest.raises(AppError) as exc:
        lease_expiry(grant, now=datetime.now(UTC))
    assert exc.value.code == "OFFLINE_POLICY_INVALID"
