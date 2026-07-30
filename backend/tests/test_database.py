from pathlib import Path

from alembic import command
from alembic.config import Config
from app.core.config import settings
from app.db.base import Base
from app.db.seed import seed_database
from app.models import AuditLog, Group, GroupSkillGrant, Skill, SkillVersion, User
from sqlalchemy import create_engine, inspect, select
from sqlalchemy.orm import Session


def test_migration_upgrade_and_downgrade(tmp_path: Path) -> None:
    database_path = tmp_path / "migration.db"
    url = f"sqlite:///{database_path.as_posix()}"
    original_url = settings.database_url
    settings.database_url = url
    config = Config("alembic.ini")
    try:
        command.upgrade(config, "head")
        tables = set(inspect(create_engine(url)).get_table_names())
        assert {
            "users",
            "groups",
            "skills",
            "skill_versions",
            "skill_templates",
            "audit_logs",
        } <= tables
        command.downgrade(config, "base")
        assert "users" not in inspect(create_engine(url)).get_table_names()
    finally:
        settings.database_url = original_url


def test_seed_is_idempotent(tmp_path: Path) -> None:
    engine = create_engine(f"sqlite:///{(tmp_path / 'seed.db').as_posix()}")
    Base.metadata.create_all(engine)
    with Session(engine) as session:
        seed_database(session)
        first = (
            len(session.scalars(select(User)).all()),
            len(session.scalars(select(Group)).all()),
            len(session.scalars(select(Skill)).all()),
            len(session.scalars(select(SkillVersion)).all()),
            len(session.scalars(select(GroupSkillGrant)).all()),
            len(session.scalars(select(AuditLog)).all()),
        )
        seed_database(session)
        second = (
            len(session.scalars(select(User)).all()),
            len(session.scalars(select(Group)).all()),
            len(session.scalars(select(Skill)).all()),
            len(session.scalars(select(SkillVersion)).all()),
            len(session.scalars(select(GroupSkillGrant)).all()),
            len(session.scalars(select(AuditLog)).all()),
        )
    assert first == second == (3, 2, 5, 5, 1, 5)
