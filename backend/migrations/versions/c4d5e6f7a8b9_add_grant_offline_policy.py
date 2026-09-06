"""add group grant offline entitlement policy (M3)

Revision ID: c4d5e6f7a8b9
Revises: b6a31d0f4c9e
Create Date: 2026-09-06 12:00:00

Adds the configurable offline TTL policy to group skill grants
(roadmap M3 / handoff §15.1): ``unlimited`` (personal default), ``ttl``
with a positive hour bound (e.g. Team 7 days = 168, Confidential 8), and
``disabled`` (Restricted: no offline window).
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

revision: str = "c4d5e6f7a8b9"
down_revision: str | None = "b6a31d0f4c9e"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None


def upgrade() -> None:
    op.add_column(
        "group_skill_grants",
        sa.Column(
            "offline_policy",
            sa.String(length=20),
            nullable=False,
            server_default="unlimited",
        ),
    )
    op.add_column(
        "group_skill_grants",
        sa.Column("offline_ttl_hours", sa.Integer(), nullable=True),
    )
    with op.batch_alter_table("group_skill_grants") as batch:
        batch.create_check_constraint(
            "ck_grant_offline_policy_allowed",
            "offline_policy IN ('unlimited', 'ttl', 'disabled')",
        )
        batch.create_check_constraint(
            "ck_grant_offline_ttl_hours_positive",
            "offline_ttl_hours IS NULL OR offline_ttl_hours >= 1",
        )
        batch.create_check_constraint(
            "ck_grant_offline_ttl_pairing",
            "(offline_policy = 'ttl') = (offline_ttl_hours IS NOT NULL)",
        )


def downgrade() -> None:
    with op.batch_alter_table("group_skill_grants") as batch:
        batch.drop_constraint("ck_grant_offline_ttl_pairing", type_="check")
        batch.drop_constraint("ck_grant_offline_ttl_hours_positive", type_="check")
        batch.drop_constraint("ck_grant_offline_policy_allowed", type_="check")
        batch.drop_column("offline_ttl_hours")
        batch.drop_column("offline_policy")
