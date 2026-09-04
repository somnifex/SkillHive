from datetime import datetime
from typing import Any, Literal, Self
from uuid import UUID

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator
from pydantic.alias_generators import to_camel

SYNC_PROTOCOL_VERSION = 1
MAX_BLOB_NEGOTIATION_ITEMS = 1024
MAX_BLOB_BYTES = 64 * 1024 * 1024
MAX_PACKAGE_BYTES = 512 * 1024 * 1024
SHA256_PATTERN = r"^sha256:[0-9a-f]{64}$"


class SyncModel(BaseModel):
    model_config = ConfigDict(
        alias_generator=to_camel,
        populate_by_name=True,
        extra="forbid",
    )


class DeviceRegisterRequest(SyncModel):
    protocol_version: Literal[1] = SYNC_PROTOCOL_VERSION
    client_instance_id: UUID
    display_name: str = Field(default="", max_length=120)
    platform: str = Field(default="", max_length=40)
    app_version: str = Field(default="", max_length=40)


class DeviceRead(SyncModel):
    protocol_version: Literal[1] = SYNC_PROTOCOL_VERSION
    device_id: UUID
    client_instance_id: UUID
    display_name: str
    platform: str
    app_version: str
    last_seen_at: datetime | None
    revoked_at: datetime | None


class SyncSkillMetadata(SyncModel):
    name: str = Field(min_length=1, max_length=120)
    slug: str = Field(min_length=2, max_length=140, pattern=r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
    description: str = Field(default="", max_length=5000)
    category: str = Field(default="", max_length=80)
    tags: list[str] = Field(default_factory=list, max_length=20)

    @field_validator("tags")
    @classmethod
    def normalize_tags(cls, values: list[str]) -> list[str]:
        return list(dict.fromkeys(value.strip()[:50] for value in values if value.strip()))


class SyncBlobDescriptor(SyncModel):
    hash: str = Field(pattern=SHA256_PATTERN)
    size_bytes: int = Field(ge=0, le=MAX_BLOB_BYTES)


class MissingBlobsRequest(SyncModel):
    protocol_version: Literal[1] = SYNC_PROTOCOL_VERSION
    objects: list[SyncBlobDescriptor] = Field(
        min_length=1,
        max_length=MAX_BLOB_NEGOTIATION_ITEMS,
    )

    @model_validator(mode="after")
    def validate_total_size_and_unique_hashes(self) -> Self:
        hashes = [item.hash for item in self.objects]
        if len(hashes) != len(set(hashes)):
            raise ValueError("blob negotiation request contains duplicate hashes")
        total = sum(item.size_bytes for item in self.objects)
        if total > MAX_PACKAGE_BYTES:
            raise ValueError("declared blob negotiation size exceeds package limit")
        return self


class MissingBlobsResponse(SyncModel):
    protocol_version: Literal[1] = SYNC_PROTOCOL_VERSION
    missing: list[SyncBlobDescriptor]


class SyncMutationRequest(SyncModel):
    protocol_version: Literal[1] = SYNC_PROTOCOL_VERSION
    device_id: UUID
    mutation_id: UUID
    operation: Literal["create", "update", "delete"]
    client_skill_id: str = Field(min_length=1, max_length=512)
    remote_skill_id: str | None = Field(default=None, max_length=36)
    base_revision: int | None = Field(default=None, ge=0)
    package_manifest_hash: str | None = Field(default=None, pattern=SHA256_PATTERN)
    metadata: SyncSkillMetadata | None = None

    @model_validator(mode="after")
    def validate_operation_shape(self) -> Self:
        if self.operation == "create":
            if self.remote_skill_id is not None:
                raise ValueError("create mutation must not include remoteSkillId")
            if self.base_revision not in {None, 0}:
                raise ValueError("create mutation baseRevision must be null or zero")
            if self.package_manifest_hash is None or self.metadata is None:
                raise ValueError("create mutation requires packageManifestHash and metadata")
        elif self.operation == "update":
            if self.remote_skill_id is None or self.base_revision is None:
                raise ValueError("update mutation requires remoteSkillId and baseRevision")
            if self.package_manifest_hash is None or self.metadata is None:
                raise ValueError("update mutation requires packageManifestHash and metadata")
        else:
            if self.remote_skill_id is None or self.base_revision is None:
                raise ValueError("delete mutation requires remoteSkillId and baseRevision")
            if self.package_manifest_hash is not None:
                raise ValueError("delete mutation must not include packageManifestHash")
        return self


class SyncMutationResult(SyncModel):
    remote_skill_id: str = Field(max_length=36)
    revision: int = Field(ge=0)
    package_manifest_hash: str | None = Field(default=None, pattern=SHA256_PATTERN)


class SyncConflictHead(SyncModel):
    remote_skill_id: str = Field(max_length=36)
    revision: int = Field(ge=0)
    package_manifest_hash: str | None = Field(default=None, pattern=SHA256_PATTERN)
    metadata: dict[str, Any] = Field(default_factory=dict)


class SyncMutationResponse(SyncModel):
    protocol_version: Literal[1] = SYNC_PROTOCOL_VERSION
    mutation_id: UUID
    status: Literal[
        "acked",
        "conflict",
        "permission_denied",
        "validation_error",
    ]
    result: SyncMutationResult | None = None
    conflict: SyncConflictHead | None = None
    error_code: str | None = Field(default=None, max_length=80)
    message: str | None = Field(default=None, max_length=1000)
    retryable: bool = False


class SyncChangeItem(SyncModel):
    sequence: int = Field(ge=1)
    resource_type: Literal["skill"] = "skill"
    resource_id: str = Field(max_length=36)
    resource_revision: int = Field(ge=0)
    operation: Literal["upsert", "delete"]
    package_manifest_hash: str | None = Field(default=None, pattern=SHA256_PATTERN)
    metadata: dict[str, Any] = Field(default_factory=dict)
    created_at: datetime


class SyncChangesResponse(SyncModel):
    protocol_version: Literal[1] = SYNC_PROTOCOL_VERSION
    changes: list[SyncChangeItem]
    next_cursor: str = Field(max_length=256)
    has_more: bool
    server_time: datetime


class SyncErrorResponse(SyncModel):
    protocol_version: Literal[1] = SYNC_PROTOCOL_VERSION
    error_code: str = Field(max_length=80)
    message: str = Field(max_length=1000)
    retryable: bool
    current_revision: int | None = Field(default=None, ge=0)
