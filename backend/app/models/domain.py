from datetime import datetime
from typing import Any

from sqlalchemy import (
    JSON,
    BigInteger,
    Boolean,
    DateTime,
    ForeignKey,
    Index,
    String,
    Text,
    UniqueConstraint,
)
from sqlalchemy.orm import Mapped, mapped_column, relationship

from app.db.base import Base, TimestampMixin, UUIDPrimaryKeyMixin, utc_now


class User(UUIDPrimaryKeyMixin, TimestampMixin, Base):
    __tablename__ = "users"

    username: Mapped[str] = mapped_column(String(50), unique=True, index=True)
    display_name: Mapped[str] = mapped_column(String(100))
    email: Mapped[str] = mapped_column(String(255), unique=True, index=True)
    password_hash: Mapped[str] = mapped_column(String(255))
    avatar_url: Mapped[str | None] = mapped_column(String(500))
    status: Mapped[str] = mapped_column(String(20), default="active", index=True)
    is_global_admin: Mapped[bool] = mapped_column(Boolean, default=False, index=True)
    last_login_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    deleted_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))


class TokenSession(UUIDPrimaryKeyMixin, TimestampMixin, Base):
    __tablename__ = "token_sessions"

    user_id: Mapped[str] = mapped_column(ForeignKey("users.id", ondelete="CASCADE"), index=True)
    refresh_jti: Mapped[str] = mapped_column(String(36), unique=True, index=True)
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), index=True)
    revoked_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))
    ip_address: Mapped[str | None] = mapped_column(String(64))
    user_agent: Mapped[str | None] = mapped_column(String(500))

    user: Mapped[User] = relationship()


class Group(UUIDPrimaryKeyMixin, TimestampMixin, Base):
    __tablename__ = "groups"

    name: Mapped[str] = mapped_column(String(120), index=True)
    description: Mapped[str] = mapped_column(Text, default="")
    avatar_url: Mapped[str | None] = mapped_column(String(500))
    group_type: Mapped[str] = mapped_column(String(20), default="personal", index=True)
    owner_id: Mapped[str] = mapped_column(ForeignKey("users.id"), index=True)
    join_policy: Mapped[str] = mapped_column(String(30), default="invite_only")
    allow_member_invite: Mapped[bool] = mapped_column(Boolean, default=False)
    status: Mapped[str] = mapped_column(String(20), default="active", index=True)
    created_by: Mapped[str] = mapped_column(ForeignKey("users.id"))
    deleted_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))

    owner: Mapped[User] = relationship(foreign_keys=[owner_id])
    members: Mapped[list["GroupMember"]] = relationship(
        back_populates="group",
        cascade="all, delete-orphan",
    )


class GroupMember(UUIDPrimaryKeyMixin, TimestampMixin, Base):
    __tablename__ = "group_members"
    __table_args__ = (
        UniqueConstraint("group_id", "user_id", name="uq_group_member"),
        Index("ix_group_member_role", "group_id", "role"),
    )

    group_id: Mapped[str] = mapped_column(
        ForeignKey("groups.id", ondelete="CASCADE"),
        index=True,
    )
    user_id: Mapped[str] = mapped_column(
        ForeignKey("users.id", ondelete="CASCADE"),
        index=True,
    )
    role: Mapped[str] = mapped_column(String(20), default="member")
    status: Mapped[str] = mapped_column(String(20), default="active", index=True)
    joined_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utc_now)
    invited_by: Mapped[str | None] = mapped_column(ForeignKey("users.id"))

    group: Mapped[Group] = relationship(back_populates="members")
    user: Mapped[User] = relationship(foreign_keys=[user_id])


class GroupInvitation(UUIDPrimaryKeyMixin, TimestampMixin, Base):
    __tablename__ = "group_invitations"
    __table_args__ = (
        UniqueConstraint("group_id", "invitee_id", "status", name="uq_active_invitation"),
    )

    group_id: Mapped[str] = mapped_column(
        ForeignKey("groups.id", ondelete="CASCADE"),
        index=True,
    )
    invitee_id: Mapped[str] = mapped_column(ForeignKey("users.id"), index=True)
    invited_by: Mapped[str] = mapped_column(ForeignKey("users.id"))
    status: Mapped[str] = mapped_column(String(20), default="pending", index=True)
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True))
    responded_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))


class GroupJoinRequest(UUIDPrimaryKeyMixin, TimestampMixin, Base):
    __tablename__ = "group_join_requests"
    __table_args__ = (
        UniqueConstraint("group_id", "user_id", "status", name="uq_active_join_request"),
    )

    group_id: Mapped[str] = mapped_column(
        ForeignKey("groups.id", ondelete="CASCADE"),
        index=True,
    )
    user_id: Mapped[str] = mapped_column(ForeignKey("users.id"), index=True)
    message: Mapped[str] = mapped_column(String(500), default="")
    status: Mapped[str] = mapped_column(String(20), default="pending", index=True)
    reviewed_by: Mapped[str | None] = mapped_column(ForeignKey("users.id"))
    reviewed_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))


class Skill(UUIDPrimaryKeyMixin, TimestampMixin, Base):
    __tablename__ = "skills"
    __table_args__ = (UniqueConstraint("owner_user_id", "slug", name="uq_owner_skill_slug"),)

    name: Mapped[str] = mapped_column(String(120), index=True)
    slug: Mapped[str] = mapped_column(String(140), index=True)
    description: Mapped[str] = mapped_column(Text, default="")
    skill_type: Mapped[str] = mapped_column(String(20), index=True)
    owner_user_id: Mapped[str | None] = mapped_column(ForeignKey("users.id"), index=True)
    category: Mapped[str] = mapped_column(String(80), default="", index=True)
    tags: Mapped[list[str]] = mapped_column(JSON, default=list)
    status: Mapped[str] = mapped_column(String(20), default="draft", index=True)
    current_version_id: Mapped[str | None] = mapped_column(String(36))
    sync_revision: Mapped[int] = mapped_column(BigInteger, default=0, nullable=False)
    current_package_hash: Mapped[str | None] = mapped_column(String(71))
    created_by: Mapped[str] = mapped_column(ForeignKey("users.id"))
    deleted_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))

    versions: Mapped[list["SkillVersion"]] = relationship(
        back_populates="skill",
        cascade="all, delete-orphan",
        foreign_keys="SkillVersion.skill_id",
    )


class SkillVersion(UUIDPrimaryKeyMixin, Base):
    __tablename__ = "skill_versions"
    __table_args__ = (
        UniqueConstraint("skill_id", "version", name="uq_skill_version"),
        UniqueConstraint("skill_id", "revision", name="uq_skill_version_revision"),
    )

    skill_id: Mapped[str] = mapped_column(
        ForeignKey("skills.id", ondelete="CASCADE"),
        index=True,
    )
    version: Mapped[str] = mapped_column(String(40))
    revision: Mapped[int] = mapped_column(BigInteger, nullable=False)
    content: Mapped[dict[str, Any]] = mapped_column(JSON)
    manifest: Mapped[dict[str, Any]] = mapped_column(JSON, default=dict)
    package_manifest_hash: Mapped[str | None] = mapped_column(String(71))
    package_size_bytes: Mapped[int | None] = mapped_column(BigInteger)
    dependency_config: Mapped[dict[str, Any]] = mapped_column(JSON, default=dict)
    change_log: Mapped[str] = mapped_column(Text, default="")
    status: Mapped[str] = mapped_column(String(20), default="draft", index=True)
    created_by: Mapped[str] = mapped_column(ForeignKey("users.id"))
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        default=utc_now,
        nullable=False,
    )

    skill: Mapped[Skill] = relationship(back_populates="versions", foreign_keys=[skill_id])


class SkillTemplate(UUIDPrimaryKeyMixin, TimestampMixin, Base):
    __tablename__ = "skill_templates"
    __table_args__ = (
        UniqueConstraint("owner_user_id", "slug", name="uq_owner_template_slug"),
        UniqueConstraint("group_id", "slug", name="uq_group_template_slug"),
        Index("ix_template_scope_status", "scope_type", "status"),
    )

    name: Mapped[str] = mapped_column(String(120), index=True)
    slug: Mapped[str] = mapped_column(String(140), index=True)
    description: Mapped[str] = mapped_column(Text, default="")
    scope_type: Mapped[str] = mapped_column(String(20), index=True)
    owner_user_id: Mapped[str | None] = mapped_column(
        ForeignKey("users.id", ondelete="CASCADE"),
        index=True,
    )
    group_id: Mapped[str | None] = mapped_column(
        ForeignKey("groups.id", ondelete="CASCADE"),
        index=True,
    )
    category: Mapped[str] = mapped_column(String(80), default="", index=True)
    tags: Mapped[list[str]] = mapped_column(JSON, default=list)
    content: Mapped[dict[str, Any]] = mapped_column(JSON)
    manifest: Mapped[dict[str, Any]] = mapped_column(JSON, default=dict)
    status: Mapped[str] = mapped_column(String(20), default="published", index=True)
    is_default: Mapped[bool] = mapped_column(Boolean, default=False, index=True)
    created_by: Mapped[str] = mapped_column(ForeignKey("users.id"))
    deleted_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))


class GroupSkillGrant(UUIDPrimaryKeyMixin, TimestampMixin, Base):
    __tablename__ = "group_skill_grants"
    __table_args__ = (UniqueConstraint("group_id", "skill_id", name="uq_group_skill_grant"),)

    group_id: Mapped[str] = mapped_column(
        ForeignKey("groups.id", ondelete="CASCADE"),
        index=True,
    )
    skill_id: Mapped[str] = mapped_column(
        ForeignKey("skills.id", ondelete="CASCADE"),
        index=True,
    )
    version_policy: Mapped[str] = mapped_column(String(20), default="latest")
    locked_version_id: Mapped[str | None] = mapped_column(ForeignKey("skill_versions.id"))
    status: Mapped[str] = mapped_column(String(20), default="active", index=True)
    granted_by: Mapped[str] = mapped_column(ForeignKey("users.id"))
    granted_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), default=utc_now)
    revoked_by: Mapped[str | None] = mapped_column(ForeignKey("users.id"))
    revoked_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True))


class AuditLog(UUIDPrimaryKeyMixin, Base):
    __tablename__ = "audit_logs"
    __table_args__ = (
        Index("ix_audit_resource", "resource_type", "resource_id"),
        Index("ix_audit_created_action", "created_at", "action"),
    )

    actor_user_id: Mapped[str | None] = mapped_column(ForeignKey("users.id"), index=True)
    action: Mapped[str] = mapped_column(String(100), index=True)
    resource_type: Mapped[str] = mapped_column(String(50), index=True)
    resource_id: Mapped[str | None] = mapped_column(String(36))
    before_data: Mapped[dict[str, Any] | None] = mapped_column(JSON)
    after_data: Mapped[dict[str, Any] | None] = mapped_column(JSON)
    ip_address: Mapped[str | None] = mapped_column(String(64))
    user_agent: Mapped[str | None] = mapped_column(String(500))
    result: Mapped[str] = mapped_column(String(20), default="success", index=True)
    error_message: Mapped[str | None] = mapped_column(String(1000))
    created_at: Mapped[datetime] = mapped_column(
        DateTime(timezone=True),
        default=utc_now,
        nullable=False,
        index=True,
    )
