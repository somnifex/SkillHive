from datetime import datetime
from typing import Any, Literal

from pydantic import BaseModel, Field, field_validator, model_validator

from app.schemas.common import ORMModel
from app.schemas.skill import SEMVER_RE, SkillContent


class TemplateCreate(BaseModel):
    name: str = Field(min_length=1, max_length=120)
    slug: str = Field(min_length=2, max_length=64, pattern=r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
    description: str = Field(default="", max_length=5000)
    scope_type: Literal["personal", "group", "global"] = "personal"
    group_id: str | None = None
    category: str = Field(default="", max_length=80)
    tags: list[str] = Field(default_factory=list, max_length=20)
    content: SkillContent = Field(default_factory=SkillContent)
    manifest: dict[str, Any] = Field(default_factory=dict)
    status: Literal["draft", "published"] = "published"

    @model_validator(mode="after")
    def validate_scope(self) -> "TemplateCreate":
        if self.scope_type == "group" and not self.group_id:
            raise ValueError("group_id is required for a group template")
        if self.scope_type != "group" and self.group_id:
            raise ValueError("group_id is only valid for a group template")
        return self

    @field_validator("tags")
    @classmethod
    def normalize_tags(cls, values: list[str]) -> list[str]:
        return list(dict.fromkeys(value.strip()[:50] for value in values if value.strip()))


class TemplateUpdate(BaseModel):
    name: str | None = Field(default=None, min_length=1, max_length=120)
    description: str | None = Field(default=None, max_length=5000)
    category: str | None = Field(default=None, max_length=80)
    tags: list[str] | None = Field(default=None, max_length=20)
    content: SkillContent | None = None
    manifest: dict[str, Any] | None = None
    status: Literal["draft", "published", "disabled"] | None = None


class TemplateInstantiate(BaseModel):
    name: str = Field(min_length=1, max_length=120)
    slug: str = Field(min_length=2, max_length=64, pattern=r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
    description: str | None = Field(default=None, max_length=5000)
    category: str | None = Field(default=None, max_length=80)
    tags: list[str] | None = Field(default=None, max_length=20)
    instructions: str | None = None
    version: str = "0.1.0"

    @field_validator("version")
    @classmethod
    def validate_version(cls, value: str) -> str:
        if not SEMVER_RE.fullmatch(value):
            raise ValueError("Version must be a semantic version such as 1.0.0")
        return value


class TemplateRead(ORMModel):
    id: str
    name: str
    slug: str
    description: str
    scope_type: str
    owner_user_id: str | None
    group_id: str | None
    category: str
    tags: list[str]
    content: dict[str, Any]
    manifest: dict[str, Any]
    status: str
    is_default: bool
    created_by: str
    created_at: datetime
    updated_at: datetime
    can_manage: bool = False
    group_name: str | None = None
