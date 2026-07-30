from datetime import datetime
from typing import Literal

from pydantic import BaseModel, Field

from app.schemas.common import ORMModel
from app.schemas.user import UserSummary


class GroupCreate(BaseModel):
    name: str = Field(min_length=1, max_length=120)
    description: str = Field(default="", max_length=5000)
    join_policy: Literal["invite_only", "approval_required", "invite_link", "public"] = (
        "invite_only"
    )
    allow_member_invite: bool = False


class GroupUpdate(BaseModel):
    name: str | None = Field(default=None, min_length=1, max_length=120)
    description: str | None = Field(default=None, max_length=5000)
    join_policy: Literal["invite_only", "approval_required", "invite_link", "public"] | None = None
    allow_member_invite: bool | None = None


class GroupRead(ORMModel):
    id: str
    name: str
    description: str
    avatar_url: str | None
    group_type: str
    owner_id: str
    join_policy: str
    allow_member_invite: bool
    status: str
    created_by: str
    created_at: datetime
    updated_at: datetime
    current_user_role: str | None = None


class MemberRead(ORMModel):
    id: str
    group_id: str
    user_id: str
    role: str
    status: str
    joined_at: datetime
    user: UserSummary | None = None


class InviteMemberRequest(BaseModel):
    identity: str = Field(min_length=1, max_length=255)


class InvitationRead(ORMModel):
    id: str
    group_id: str
    invitee_id: str
    invited_by: str
    status: str
    expires_at: datetime
    created_at: datetime
    group_name: str | None = None


class JoinRequestCreate(BaseModel):
    message: str = Field(default="", max_length=500)


class JoinRequestReview(BaseModel):
    decision: Literal["approved", "rejected"]


class JoinRequestRead(ORMModel):
    id: str
    group_id: str
    user_id: str
    message: str
    status: str
    reviewed_by: str | None
    reviewed_at: datetime | None
    created_at: datetime
    user: UserSummary | None = None


class MemberRoleUpdate(BaseModel):
    role: Literal["admin", "member"]


class TransferOwnershipRequest(BaseModel):
    new_owner_user_id: str


class AdminGroupStatusUpdate(BaseModel):
    status: Literal["active", "archived", "deleted"]
