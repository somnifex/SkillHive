"""add M2 sync foundation

Revision ID: b6a31d0f4c9e
Revises: 7f4c2b8a91de
Create Date: 2026-09-04 14:10:00
"""

from collections.abc import Sequence

import sqlalchemy as sa
from alembic import op

revision: str = "b6a31d0f4c9e"
down_revision: str | None = "7f4c2b8a91de"
branch_labels: str | Sequence[str] | None = None
depends_on: str | Sequence[str] | None = None

_SHA256_LENGTH = 71


def upgrade() -> None:
    op.add_column(
        "skills",
        sa.Column("sync_revision", sa.BigInteger(), server_default=sa.text("0"), nullable=True),
    )
    op.add_column(
        "skills",
        sa.Column("current_package_hash", sa.String(length=_SHA256_LENGTH), nullable=True),
    )
    op.add_column(
        "skill_versions",
        sa.Column("revision", sa.BigInteger(), nullable=True),
    )
    op.add_column(
        "skill_versions",
        sa.Column("package_manifest_hash", sa.String(length=_SHA256_LENGTH), nullable=True),
    )
    op.add_column(
        "skill_versions",
        sa.Column("package_size_bytes", sa.BigInteger(), nullable=True),
    )

    _backfill_skill_revisions()

    with op.batch_alter_table("skills") as batch:
        batch.alter_column(
            "sync_revision",
            existing_type=sa.BigInteger(),
            nullable=False,
            server_default=sa.text("1"),
        )
        batch.create_check_constraint(
            "ck_skill_sync_revision_positive",
            "sync_revision >= 1",
        )
    with op.batch_alter_table("skill_versions") as batch:
        batch.alter_column("revision", existing_type=sa.BigInteger(), nullable=False)
        batch.create_unique_constraint(
            "uq_skill_version_revision",
            ["skill_id", "revision"],
        )
        batch.create_check_constraint(
            "ck_skill_version_revision_positive",
            "revision >= 1",
        )
        batch.create_check_constraint(
            "ck_skill_version_package_size_nonnegative",
            "package_size_bytes IS NULL OR package_size_bytes >= 0",
        )

    op.create_table(
        "devices",
        sa.Column("user_id", sa.String(length=36), nullable=False),
        sa.Column("client_instance_id", sa.String(length=36), nullable=False),
        sa.Column("display_name", sa.String(length=120), nullable=False),
        sa.Column("platform", sa.String(length=40), nullable=False),
        sa.Column("app_version", sa.String(length=40), nullable=False),
        sa.Column("last_seen_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("revoked_at", sa.DateTime(timezone=True), nullable=True),
        sa.Column("id", sa.String(length=36), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("updated_at", sa.DateTime(timezone=True), nullable=False),
        sa.ForeignKeyConstraint(["user_id"], ["users.id"], ondelete="CASCADE"),
        sa.PrimaryKeyConstraint("id"),
        sa.UniqueConstraint(
            "user_id",
            "client_instance_id",
            name="uq_device_user_client_instance",
        ),
    )
    op.create_index("ix_devices_user_id", "devices", ["user_id"])
    op.create_index("ix_devices_revoked_at", "devices", ["revoked_at"])
    op.create_index("ix_device_user_active", "devices", ["user_id", "revoked_at"])

    op.create_table(
        "skill_blob_objects",
        sa.Column("hash", sa.String(length=_SHA256_LENGTH), nullable=False),
        sa.Column("size_bytes", sa.BigInteger(), nullable=False),
        sa.Column("storage_key", sa.String(length=1000), nullable=False),
        sa.Column("storage_backend", sa.String(length=40), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.Column("verified_at", sa.DateTime(timezone=True), nullable=True),
        sa.CheckConstraint("size_bytes >= 0", name="ck_skill_blob_size_nonnegative"),
        sa.PrimaryKeyConstraint("hash"),
        sa.UniqueConstraint("storage_key", name="uq_skill_blob_storage_key"),
    )

    op.create_table(
        "sync_mutation_receipts",
        sa.Column("user_id", sa.String(length=36), nullable=False),
        sa.Column("device_id", sa.String(length=36), nullable=False),
        sa.Column("mutation_id", sa.String(length=36), nullable=False),
        sa.Column("operation", sa.String(length=20), nullable=False),
        sa.Column("resource_type", sa.String(length=40), nullable=False),
        sa.Column("resource_id", sa.String(length=36), nullable=True),
        sa.Column("result_code", sa.String(length=64), nullable=False),
        sa.Column("result_revision", sa.BigInteger(), nullable=True),
        sa.Column("response_payload", sa.JSON(), nullable=False),
        sa.Column("id", sa.String(length=36), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.CheckConstraint(
            "result_revision IS NULL OR result_revision >= 1",
            name="ck_sync_receipt_revision_positive",
        ),
        sa.ForeignKeyConstraint(["device_id"], ["devices.id"], ondelete="CASCADE"),
        sa.ForeignKeyConstraint(["user_id"], ["users.id"], ondelete="CASCADE"),
        sa.PrimaryKeyConstraint("id"),
        sa.UniqueConstraint(
            "user_id",
            "device_id",
            "mutation_id",
            name="uq_sync_receipt_user_device_mutation",
        ),
    )
    op.create_index("ix_sync_mutation_receipts_user_id", "sync_mutation_receipts", ["user_id"])
    op.create_index("ix_sync_mutation_receipts_device_id", "sync_mutation_receipts", ["device_id"])
    op.create_index(
        "ix_sync_mutation_receipts_resource_id",
        "sync_mutation_receipts",
        ["resource_id"],
    )
    op.create_index(
        "ix_sync_receipt_resource",
        "sync_mutation_receipts",
        ["resource_type", "resource_id"],
    )
    op.create_index(
        "ix_sync_mutation_receipts_created_at",
        "sync_mutation_receipts",
        ["created_at"],
    )

    sequence_type = sa.BigInteger().with_variant(sa.Integer(), "sqlite")
    op.create_table(
        "sync_change_log",
        sa.Column("sequence", sequence_type, autoincrement=True, nullable=False),
        sa.Column("resource_type", sa.String(length=40), nullable=False),
        sa.Column("resource_id", sa.String(length=36), nullable=False),
        sa.Column("resource_revision", sa.BigInteger(), nullable=False),
        sa.Column("operation", sa.String(length=20), nullable=False),
        sa.Column("owner_user_id", sa.String(length=36), nullable=True),
        sa.Column("package_manifest_hash", sa.String(length=_SHA256_LENGTH), nullable=True),
        sa.Column("metadata_payload", sa.JSON(), nullable=False),
        sa.Column("created_at", sa.DateTime(timezone=True), nullable=False),
        sa.CheckConstraint(
            "resource_revision >= 1",
            name="ck_sync_change_revision_positive",
        ),
        sa.ForeignKeyConstraint(["owner_user_id"], ["users.id"], ondelete="SET NULL"),
        sa.PrimaryKeyConstraint("sequence"),
    )
    op.create_index("ix_sync_change_log_owner_user_id", "sync_change_log", ["owner_user_id"])
    op.create_index("ix_sync_change_log_created_at", "sync_change_log", ["created_at"])
    op.create_index(
        "ix_sync_change_resource",
        "sync_change_log",
        ["resource_type", "resource_id", "resource_revision"],
    )
    op.create_index(
        "ix_sync_change_owner_sequence",
        "sync_change_log",
        ["owner_user_id", "sequence"],
    )

    _backfill_change_feed_baseline()


def downgrade() -> None:
    op.drop_index("ix_sync_change_owner_sequence", table_name="sync_change_log")
    op.drop_index("ix_sync_change_resource", table_name="sync_change_log")
    op.drop_index("ix_sync_change_log_created_at", table_name="sync_change_log")
    op.drop_index("ix_sync_change_log_owner_user_id", table_name="sync_change_log")
    op.drop_table("sync_change_log")

    op.drop_index("ix_sync_mutation_receipts_created_at", table_name="sync_mutation_receipts")
    op.drop_index("ix_sync_receipt_resource", table_name="sync_mutation_receipts")
    op.drop_index("ix_sync_mutation_receipts_resource_id", table_name="sync_mutation_receipts")
    op.drop_index("ix_sync_mutation_receipts_device_id", table_name="sync_mutation_receipts")
    op.drop_index("ix_sync_mutation_receipts_user_id", table_name="sync_mutation_receipts")
    op.drop_table("sync_mutation_receipts")

    op.drop_table("skill_blob_objects")

    op.drop_index("ix_device_user_active", table_name="devices")
    op.drop_index("ix_devices_revoked_at", table_name="devices")
    op.drop_index("ix_devices_user_id", table_name="devices")
    op.drop_table("devices")

    with op.batch_alter_table("skill_versions") as batch:
        batch.drop_constraint("ck_skill_version_package_size_nonnegative", type_="check")
        batch.drop_constraint("ck_skill_version_revision_positive", type_="check")
        batch.drop_constraint("uq_skill_version_revision", type_="unique")
        batch.drop_column("package_size_bytes")
        batch.drop_column("package_manifest_hash")
        batch.drop_column("revision")
    with op.batch_alter_table("skills") as batch:
        batch.drop_constraint("ck_skill_sync_revision_positive", type_="check")
        batch.drop_column("current_package_hash")
        batch.drop_column("sync_revision")


def _backfill_skill_revisions() -> None:
    connection = op.get_bind()
    skill_ids = list(connection.execute(sa.text("SELECT id FROM skills ORDER BY id")).scalars())
    for skill_id in skill_ids:
        version_ids = list(
            connection.execute(
                sa.text(
                    """
                    SELECT id
                    FROM skill_versions
                    WHERE skill_id = :skill_id
                    ORDER BY created_at ASC, id ASC
                    """
                ),
                {"skill_id": skill_id},
            ).scalars()
        )
        for revision_value, version_id in enumerate(version_ids, start=1):
            connection.execute(
                sa.text("UPDATE skill_versions SET revision = :revision WHERE id = :version_id"),
                {"revision": revision_value, "version_id": version_id},
            )
        baseline_revision = max(1, len(version_ids))
        connection.execute(
            sa.text("UPDATE skills SET sync_revision = :revision WHERE id = :skill_id"),
            {"revision": baseline_revision, "skill_id": skill_id},
        )


def _backfill_change_feed_baseline() -> None:
    connection = op.get_bind()
    rows = connection.execute(
        sa.text(
            """
            SELECT id, owner_user_id, name, slug, description, skill_type, category,
                   tags, status, sync_revision, current_package_hash, updated_at
            FROM skills
            ORDER BY created_at ASC, id ASC
            """
        )
    ).mappings()

    change_log = sa.table(
        "sync_change_log",
        sa.column("resource_type", sa.String(length=40)),
        sa.column("resource_id", sa.String(length=36)),
        sa.column("resource_revision", sa.BigInteger()),
        sa.column("operation", sa.String(length=20)),
        sa.column("owner_user_id", sa.String(length=36)),
        sa.column("package_manifest_hash", sa.String(length=_SHA256_LENGTH)),
        sa.column("metadata_payload", sa.JSON()),
        sa.column("created_at", sa.DateTime(timezone=True)),
    )
    for row in rows:
        connection.execute(
            change_log.insert().values(
                resource_type="skill",
                resource_id=row["id"],
                resource_revision=row["sync_revision"],
                operation="delete" if row["status"] == "deleted" else "upsert",
                owner_user_id=row["owner_user_id"],
                package_manifest_hash=row["current_package_hash"],
                metadata_payload={
                    "name": row["name"],
                    "slug": row["slug"],
                    "description": row["description"],
                    "skill_type": row["skill_type"],
                    "category": row["category"],
                    "tags": row["tags"],
                    "status": row["status"],
                },
                created_at=row["updated_at"],
            )
        )
