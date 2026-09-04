from sqlalchemy import func, select
from sqlalchemy.orm import Session

from app.models import AuditLog, Skill, SkillVersion, User
from app.services.skill_mutations import SkillMutationService


def _user(session: Session) -> User:
    user = User(
        username="mutation-owner",
        display_name="Mutation Owner",
        email="mutation-owner@example.test",
        password_hash="not-used-by-this-test",
        status="active",
        is_global_admin=False,
    )
    session.add(user)
    session.commit()
    return user


def test_domain_mutation_does_not_commit_caller_transaction(db_session: Session) -> None:
    user = _user(db_session)
    mutations = SkillMutationService(db_session, user.id)

    skill, version = mutations.create_skill(
        name="Transaction Boundary",
        slug="transaction-boundary",
        description="",
        skill_type="private",
        owner_user_id=user.id,
        category="",
        tags=[],
        skill_status="draft",
        version="0.1.0",
        content={"skill_markdown": "# Boundary\n"},
        manifest={"name": "transaction-boundary", "schema_version": 1},
        dependency_config={},
        change_log="Initial version",
        version_status="draft",
        audit_action="private_skill.created",
    )

    assert db_session.get(Skill, skill.id) is skill
    assert db_session.get(SkillVersion, version.id) is version
    assert db_session.scalar(select(func.count()).select_from(AuditLog)) == 1

    # The domain layer must not have committed. A future sync handler needs to
    # be able to append receipt/change-feed rows and atomically roll everything
    # back if any part of that larger transaction fails.
    db_session.rollback()

    assert db_session.get(Skill, skill.id) is None
    assert db_session.get(SkillVersion, version.id) is None
    assert db_session.scalar(select(func.count()).select_from(AuditLog)) == 0


def test_domain_mutation_commits_when_transaction_owner_commits(db_session: Session) -> None:
    user = _user(db_session)
    mutations = SkillMutationService(db_session, user.id)

    skill, version = mutations.create_skill(
        name="Durable Boundary",
        slug="durable-boundary",
        description="",
        skill_type="private",
        owner_user_id=user.id,
        category="",
        tags=[],
        skill_status="draft",
        version="0.1.0",
        content={"skill_markdown": "# Durable\n"},
        manifest={"name": "durable-boundary", "schema_version": 1},
        dependency_config={},
        change_log="Initial version",
        version_status="draft",
        audit_action="private_skill.created",
    )
    db_session.commit()

    assert db_session.get(Skill, skill.id) is not None
    assert db_session.get(SkillVersion, version.id) is not None
    audit = db_session.scalar(
        select(AuditLog).where(
            AuditLog.resource_type == "skill",
            AuditLog.resource_id == skill.id,
            AuditLog.action == "private_skill.created",
        )
    )
    assert audit is not None
