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


def _create_skill(mutations: SkillMutationService, *, slug: str) -> tuple[Skill, SkillVersion]:
    return mutations.create_skill(
        name="Transaction Boundary",
        slug=slug,
        description="",
        skill_type="private",
        owner_user_id=mutations.actor_user_id,
        category="",
        tags=[],
        skill_status="draft",
        version="0.1.0",
        content={"skill_markdown": "# Boundary\n"},
        manifest={"name": slug, "schema_version": 1},
        dependency_config={},
        change_log="Initial version",
        version_status="draft",
        audit_action="private_skill.created",
    )


def test_domain_mutation_does_not_commit_caller_transaction(db_session: Session) -> None:
    user = _user(db_session)
    mutations = SkillMutationService(db_session, user.id)

    skill, version = _create_skill(mutations, slug="transaction-boundary")

    assert skill.sync_revision == 1
    assert version.revision == 1
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

    skill, version = _create_skill(mutations, slug="durable-boundary")
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


def test_visible_mutations_advance_one_technical_revision_each(db_session: Session) -> None:
    user = _user(db_session)
    mutations = SkillMutationService(db_session, user.id)
    skill, first = _create_skill(mutations, slug="revision-order")

    assert skill.sync_revision == 1
    assert first.revision == 1

    mutations.update_skill(
        skill,
        updates={"name": "Renamed"},
        audit_action="private_skill.updated",
    )
    assert skill.sync_revision == 2
    assert skill.current_version_id == first.id

    second = mutations.create_version(
        skill,
        version="0.2.0",
        content={"skill_markdown": "# Revision 3\n"},
        manifest={"name": "revision-order", "schema_version": 1},
        dependency_config={},
        change_log="New content",
        version_status="draft",
        audit_action="private_skill.version_created",
    )
    assert skill.sync_revision == 3
    assert second.revision == 3
    assert skill.current_version_id == second.id

    mutations.set_status(skill, "published", audit_action="private_skill.published")
    assert skill.sync_revision == 4


def test_metadata_and_content_update_share_one_revision(db_session: Session) -> None:
    user = _user(db_session)
    mutations = SkillMutationService(db_session, user.id)
    skill, _ = _create_skill(mutations, slug="single-revision")

    version = mutations.update_skill(
        skill,
        updates={"name": "Changed Name"},
        audit_action="private_skill.updated",
        content={"skill_markdown": "# Changed\n"},
        version="0.2.0",
        change_log="Changed metadata and content",
    )

    assert version is not None
    assert skill.sync_revision == 2
    assert version.revision == 2


def test_noop_metadata_update_does_not_advance_revision(db_session: Session) -> None:
    user = _user(db_session)
    mutations = SkillMutationService(db_session, user.id)
    skill, _ = _create_skill(mutations, slug="no-op-revision")

    mutations.update_skill(
        skill,
        updates={"name": skill.name},
        audit_action="private_skill.updated",
    )

    assert skill.sync_revision == 1
