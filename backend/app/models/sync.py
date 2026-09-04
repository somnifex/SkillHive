from datetime import datetime
from typing import Any

from sqlalchemy import (
    BigInteger,
    CheckConstraint,
    DateTime,
    ForeignKey,
    Index,
    Integer,
    JSON,
    String,
    UniqueConstraint,
)
from sqlalchemy.orm import Mapped, mapped_column, relationship

from app.db.base import Base, TimestampMixin, UUIDPrimaryKeyMixin, utc_now
from app.models.domain import User


class Device(UUIDPrimaryKeyMixin, TimestampMixin, Base):
    __tablename__ = "devices"
    __table_args__ = (
        UniqueConstraint("user_id", "client_instance_id", name="uq_device_user_client_instance"),
        Index("ix_device_user_active", "user_id", "revoked_at"),
    )

    user_id: Mapped[str] = mapped_column(
        ForeignKey("users.id", ondelete="CASCADE"),
        index=True,
    )
    client_instance_id: Mapped[str] = mapped_column(String(36))
    display_name: Mapped[str] = mapped_column(String(120), default="")
    platform: Mapped[str] = mapped_column(String(40), default="")
    app_version: Mapped[str] = mapped_column(String(40), default="")
    last_seen_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    revoked_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), index=True)

    user: Mapped[User] = relationship()


class SkillBlobObject(Base):
    __tablename__ = "skill_blob_objects"
    __table_args__ = (
        CheckConstraint("size_bytes >= 0", name="ck_skill_blob_size_nonnegative"),
        UniqueConstraint("storage_key", name="uq_skill_blob_storage_key"),
    )

    hash: Mapped[str] = mapped_column(String(71), primary_key=True)
    size_bytes: Mapped[int] = mapped_column(BigInteger, nullable=False)
    storage_key: Mapped[str] = mapped_column(String(1000), nullable=False)
    storage_backend: Mapped[str] = mapped_column(String(40), nullable=False)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        default=utc_now,
        nullable=False,
    )
    verified_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))


class SyncMutationReceipt(UUIDPrimaryKeyMixin, Base):
    __tablename__ = "sync_mutation_receipts"
    __table_args__ = (
        UniqueConstraint(
            "user_id",
            "device_id",
            "mutation_id",
            name="uq_sync_receipt_user_device_mutation",
        ),
        CheckConstraint(
            "result_revision IS NULL OR result_revision >= 1",
            name="ck_sync_receipt_revision_positive",
        ),
        Index("ix_sync_receipt_resource", "resource_type", "resource_id"),
    )

    user_id: Mapped[str] = mapped_column(
        ForeignKey("users.id", ondelete="CASCADE"),
        index=True,
    )
    device_id: Mapped[str] = mapped_column(
        ForeignKey("devices.id", ondelete="CASCADE"),
        index=True,
    )
    mutation_id: Mapped[str] = mapped_column(String(36))
    operation: Mapped[str] = mapped_column(String(20))
    resource_type: Mapped[str] = mapped_column(String(40))
    resource_id: Mapped[str | None] = mapped_column(String(36), index=True)
    result_code: Mapped[str] = mapped_column(String(64))
    result_revision: Mapped[int | None] = mapped_column(BigInteger)
    response_payload: Mapped[dict[str, Any]] = mapped_column(JSON, default=dict)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        default=utc_now,
        nullable=False,
        index=True,
    )


class SyncChangeLog(Base):
    __tablename__ = "sync_change_log"
    __table_args__ = (
        CheckConstraint(
            "resource_revision >= 1",
            name="ck_sync_change_revision_positive",
        ),
        Index("ix_sync_change_resource", "resource_type", "resource_id", "resource_revision"),
        Index("ix_sync_change_owner_sequence", "owner_user_id", "sequence"),
    )

    # SQLite only auto-increments a primary key whose storage type is INTEGER.
    # PostgreSQL/MySQL retain BIGINT semantics through the type variant.
    sequence: Mapped[int] = mapped_column(
        BigInteger().with_variant(Integer, "sqlite"),
        primary_key=True,
        autoincrement=True,
    )
    resource_type: Mapped[str] = mapped_column(String(40))
    resource_id: Mapped[str] = mapped_column(String(36))
    resource_revision: Mapped[int] = mapped_column(BigInteger)
    operation: Mapped[str] = mapped_column(String(20))
    owner_user_id: Mapped[str | None] = mapped_column(
        ForeignKey("users.id", ondelete="SET NULL"),
        index=True,
    )
    package_manifest_hash: Mapped[str | None] = mapped_column(String(71))
    metadata_payload: Mapped[dict[str, Any]] = mapped_column(JSON, default=dict)
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        default=utc_now,
        nullable=False,
        index=True,
    )
