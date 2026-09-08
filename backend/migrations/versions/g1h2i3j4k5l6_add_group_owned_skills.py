"""add first-class group-owned Skills

Revision ID: g1h2i3j4k5l6
Revises: a9b0c1d2e3f4
Create Date: 2026-09-08 18:00:00
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op


revision: str = "g1h2i3j4k5l6"
down_revision: str | None = "a9b0c1d2e3f4"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    with op.batch_alter_table("skills") as batch:
        batch.add_column(
            sa.Column(
                "group_id",
                sa.String(length=36),
                nullable=True,
            )
        )
        batch.create_foreign_key(
            "fk_skills_group_id_groups",
            "groups",
            ["group_id"],
            ["id"],
        )
        batch.create_index("ix_skills_group_id", ["group_id"])
        batch.create_unique_constraint("uq_group_skill_slug", ["group_id", "slug"])


def downgrade() -> None:
    with op.batch_alter_table("skills") as batch:
        batch.drop_constraint("uq_group_skill_slug", type_="unique")
        batch.drop_index("ix_skills_group_id")
        batch.drop_column("group_id")
