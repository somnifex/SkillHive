import re
from datetime import datetime
from typing import Any, Literal, Self

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator

from app.schemas.common import ORMModel

SEMVER_RE = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$")


class SkillContent(BaseModel):
    system_prompt: str = ""
    instructions: str = ""
    examples: list[dict[str, Any]] = Field(default_factory=list)
    tools: list[str] = Field(default_factory=list)
    parameters: dict[str, Any] = Field(default_factory=dict)
    skill_markdown: str = ""


class SkillCreate(BaseModel):
    model_config = ConfigDict(
        json_schema_extra={
            "examples": [
                {
                    "name": "Research Summary",
                    "slug": "research-summary",
                    "description": "Summarize research papers",
                    "category": "Research",
                    "tags": ["papers", "summary"],
                    "content": {"instructions": "Summarize the background, method, and findings."},
                    "version": "0.1.0",
                }
            ]
        }
    )

    name: str = Field(min_length=1, max_length=120)
    slug: str = Field(min_length=2, max_length=140, pattern=r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
    description: str = Field(default="", max_length=5000)
    category: str = Field(default="", max_length=80)
    tags: list[str] = Field(default_factory=list, max_length=20)
    content: SkillContent = Field(default_factory=SkillContent)
    version: str = "0.1.0"
    change_log: str = Field(default="Initial version", max_length=2000)

    @field_validator("version")
    @classmethod
    def validate_version(cls, value: str) -> str:
        if not SEMVER_RE.fullmatch(value):
            raise ValueError("Version must be a semantic version such as 1.0.0")
        return value

    @field_validator("tags")
    @classmethod
    def normalize_tags(cls, values: list[str]) -> list[str]:
        return list(dict.fromkeys(value.strip()[:50] for value in values if value.strip()))


class SkillUpdate(BaseModel):
    name: str | None = Field(default=None, min_length=1, max_length=120)
    description: str | None = Field(default=None, max_length=5000)
    category: str | None = Field(default=None, max_length=80)
    tags: list[str] | None = Field(default=None, max_length=20)
    status: Literal["draft", "published", "disabled", "archived"] | None = None
    content: SkillContent | None = None
    version: str | None = None
    change_log: str = Field(default="Updated content", max_length=2000)

    @field_validator("version")
    @classmethod
    def validate_version(cls, value: str | None) -> str | None:
        if value is not None and not SEMVER_RE.fullmatch(value):
            raise ValueError("Version must be a semantic version such as 1.0.0")
        return value


class SkillVersionCreate(BaseModel):
    version: str
    content: SkillContent
    manifest: dict[str, Any] = Field(default_factory=dict)
    dependency_config: dict[str, Any] = Field(default_factory=dict)
    change_log: str = Field(default="", max_length=2000)
    status: Literal["draft", "published"] = "draft"
    # Docker-style labels; must be unique within one skill (service-enforced).
    tags: list[str] = Field(default_factory=list, max_length=16)


    @field_validator("version")
    @classmethod
    def validate_version(cls, value: str) -> str:
        if not SEMVER_RE.fullmatch(value):
            raise ValueError("Version must be a semantic version such as 1.0.0")
        return value


class VersionTagsUpdate(BaseModel):
    """Replaces the tag set of one version (docker-style labels)."""

    tags: list[str] = Field(max_length=16)

    @field_validator("tags")
    @classmethod
    def validate_tags(cls, value: list[str]) -> list[str]:
        cleaned: list[str] = []
        for raw in value:
            tag = raw.strip()
            if not tag:
                raise ValueError("Tags must not be empty")
            if len(tag) > 64:
                raise ValueError("Tags must be at most 64 characters")
            if tag not in cleaned:
                cleaned.append(tag)
        return cleaned


class SkillVersionRead(ORMModel):
    id: str
    skill_id: str
    version: str
    revision: int
    content: dict[str, Any]
    manifest: dict[str, Any]
    package_manifest_hash: str | None
    package_size_bytes: int | None
    dependency_config: dict[str, Any]
    change_log: str
    tags: list[str] = Field(default_factory=list)
    status: str
    created_by: str
    created_at: datetime

    @field_validator("tags", mode="before")
    @classmethod
    def normalize_tags(cls, value: object) -> list[str]:
        # Rows written before the migration carry NULL.
        if value is None:
            return []
        if isinstance(value, list):
            return [str(tag) for tag in value]
        return []


class VersionRollbackRequest(BaseModel):
    version: str
    change_log: str = Field(default="", max_length=2000)


class SkillRead(ORMModel):
    id: str
    name: str
    slug: str
    description: str
    skill_type: str
    owner_user_id: str | None
    category: str
    tags: list[str]
    status: str
    current_version_id: str | None
    sync_revision: int
    current_package_hash: str | None
    created_by: str
    created_at: datetime
    updated_at: datetime
    # Set only while the skill sits in the trash (status == 'deleted').
    deleted_at: datetime | None = None
    current_version: SkillVersionRead | None = None


class GlobalSkillCreate(SkillCreate):
    version: str = "0.1.0"


class GlobalSkillUpdate(SkillUpdate):
    status: Literal["draft", "published", "disabled", "archived"] | None = None


class PublishSkillRequest(BaseModel):
    version_id: str | None = None


OfflinePolicy = Literal["unlimited", "ttl", "disabled"]


class GroupSkillGrantCreate(BaseModel):
    model_config = ConfigDict(
        json_schema_extra={
            "examples": [
                {"version_policy": "latest"},
                {
                    "version_policy": "locked",
                    "locked_version_id": "00000000-0000-0000-0000-000000000000",
                },
                {"version_policy": "latest", "offline_policy": "ttl", "offline_ttl_hours": 168},
            ]
        }
    )

    version_policy: Literal["latest", "locked"] = "latest"
    locked_version_id: str | None = None
    offline_policy: OfflinePolicy = "unlimited"
    offline_ttl_hours: int | None = Field(default=None, ge=1, le=24 * 365)

    @model_validator(mode="after")
    def validate_offline_ttl_pairing(self) -> Self:
        if self.offline_policy == "ttl" and self.offline_ttl_hours is None:
            raise ValueError("offline_policy 'ttl' requires offline_ttl_hours")
        if self.offline_policy != "ttl" and self.offline_ttl_hours is not None:
            raise ValueError("offline_ttl_hours is only valid with offline_policy 'ttl'")
        return self


class GroupSkillGrantUpdate(BaseModel):
    version_policy: Literal["latest", "locked"] | None = None
    locked_version_id: str | None = None
    offline_policy: OfflinePolicy | None = None
    offline_ttl_hours: int | None = Field(default=None, ge=1, le=24 * 365)
    status: Literal["active", "disabled"] | None = None

    @model_validator(mode="after")
    def validate_offline_ttl_pairing(self) -> Self:
        # ``None`` on both fields means "not set"; explicit reset of a TTL
        # goes through offline_policy = "unlimited"/"disabled".
        if self.offline_policy is None and self.offline_ttl_hours is not None:
            raise ValueError("offline_ttl_hours requires offline_policy 'ttl' in the same update")
        if self.offline_policy == "ttl" and self.offline_ttl_hours is None:
            raise ValueError("offline_policy 'ttl' requires offline_ttl_hours")
        return self


class GroupSkillGrantRead(ORMModel):
    id: str
    group_id: str
    skill_id: str
    version_policy: str
    locked_version_id: str | None
    offline_policy: str
    offline_ttl_hours: int | None
    status: str
    granted_by: str
    granted_at: datetime
    revoked_by: str | None
    revoked_at: datetime | None
    skill: SkillRead | None = None
    effective_version: SkillVersionRead | None = None
