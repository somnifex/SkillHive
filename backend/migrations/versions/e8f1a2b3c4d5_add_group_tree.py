"""add group tree (parent_id self-reference)

Revision ID: e8f1a2b3c4d5
Revises: c4d5e6f7a8b9
Create Date: 2026-09-07 10:00:00

Adds tree nesting to groups (docs/architecture/group-tree-and-admin-console.md
§1): a nullable self-referencing ``parent_id`` (NULL = root level), an index
for child lookups and a no-self-parent CHECK. The change is purely additive —
every existing row becomes a root.
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

revision: str = "e8f1a2b3c4d5"
down_revision: str | None = "c4d5e6f7a8b9"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    with op.batch_alter_table("groups") as batch:
        batch.add_column(sa.Column("parent_id", sa.String(length=36), nullable=True))
        batch.create_index("ix_groups_parent_id", ["parent_id"])
        batch.create_check_constraint(
            "ck_groups_parent_not_self",
            "parent_id IS NULL OR parent_id != id",
        )


def downgrade() -> None:
    with op.batch_alter_table("groups") as batch:
        batch.drop_constraint("ck_groups_parent_not_self", type_="check")
        batch.drop_index("ix_groups_parent_id")
        batch.drop_column("parent_id")
