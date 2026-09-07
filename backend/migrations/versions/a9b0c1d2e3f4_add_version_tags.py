"""add version tags column

Revision ID: a9b0c1d2e3f4
Revises: f7a8b9c0d1e2
Create Date: 2026-09-07 23:00:00

Docker-style labels on SkillVersion rows: a version can carry several short
tags (e.g. "stable", "v1-prod") that users select in UIs. Uniqueness of a tag
inside one skill is enforced by the service layer, not the DB, so the
migration only adds the nullable JSON column and backfills it.
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

revision: str = "a9b0c1d2e3f4"
down_revision: str | None = "f7a8b9c0d1e2"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    with op.batch_alter_table("skill_versions") as batch:
        batch.add_column(sa.Column("tags", sa.JSON(), nullable=True))
    # The backfill must run outside the batch: SQLite batch mode rebuilds the
    # table at context exit, so in-batch statements would not see the column.
    op.execute("UPDATE skill_versions SET tags = '[]' WHERE tags IS NULL")


def downgrade() -> None:
    with op.batch_alter_table("skill_versions") as batch:
        batch.drop_column("tags")
