from typing import Annotated

from fastapi import APIRouter, Depends, Query, Response
from sqlalchemy.orm import Session

from app.db.session import get_db
from app.permissions.dependencies import CurrentUser
from app.schemas.common import MessageResponse, Page
from app.schemas.group import (
    GroupCreate,
    GroupRead,
    GroupUpdate,
    InvitationRead,
    InviteMemberRequest,
    JoinRequestCreate,
    JoinRequestRead,
    JoinRequestReview,
    MemberRead,
    MemberRoleUpdate,
    TransferOwnershipRequest,
)
from app.services.groups import GroupService

router = APIRouter(prefix="/groups", tags=["groups"])


@router.get("", response_model=Page[GroupRead])
def list_groups(
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
    page: Annotated[int, Query(ge=1)] = 1,
    page_size: Annotated[int, Query(ge=1, le=100)] = 20,
    managed_only: bool = False,
) -> Page[GroupRead]:
    return GroupService(session, user).list_page(
        page=page,
        page_size=page_size,
        managed_only=managed_only,
    )


@router.post("", response_model=GroupRead, status_code=201)
def create_group(
    data: GroupCreate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> GroupRead:
    return GroupService(session, user).create(data)


@router.get("/invitations", response_model=list[InvitationRead])
def my_invitations(
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> list[InvitationRead]:
    return GroupService(session, user).my_invitations()


@router.post("/invitations/{invitation_id}/accept", response_model=MessageResponse)
def accept_invitation(
    invitation_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> MessageResponse:
    GroupService(session, user).respond_invitation(invitation_id, True)
    return MessageResponse(message="Invitation accepted.")


@router.post("/invitations/{invitation_id}/decline", response_model=MessageResponse)
def decline_invitation(
    invitation_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> MessageResponse:
    GroupService(session, user).respond_invitation(invitation_id, False)
    return MessageResponse(message="Invitation declined.")


@router.get("/{group_id}", response_model=GroupRead)
def get_group(
    group_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> GroupRead:
    return GroupService(session, user).get(group_id)


@router.patch("/{group_id}", response_model=GroupRead)
def update_group(
    group_id: str,
    data: GroupUpdate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> GroupRead:
    return GroupService(session, user).update(group_id, data)


@router.delete("/{group_id}", status_code=204)
def dissolve_group(
    group_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> Response:
    GroupService(session, user).dissolve(group_id)
    return Response(status_code=204)


@router.get("/{group_id}/members", response_model=list[MemberRead])
def list_members(
    group_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> list[MemberRead]:
    return GroupService(session, user).members(group_id)


@router.post("/{group_id}/members/invite", response_model=InvitationRead, status_code=201)
def invite_member(
    group_id: str,
    data: InviteMemberRequest,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> InvitationRead:
    return GroupService(session, user).invite(group_id, data.identity)


@router.patch("/{group_id}/members/{user_id}", response_model=MemberRead)
def update_member_role(
    group_id: str,
    user_id: str,
    data: MemberRoleUpdate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> MemberRead:
    return GroupService(session, user).set_role(group_id, user_id, data.role)


@router.delete("/{group_id}/members/{user_id}", status_code=204)
def remove_member(
    group_id: str,
    user_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> Response:
    GroupService(session, user).remove_member(group_id, user_id)
    return Response(status_code=204)


@router.post("/{group_id}/join-requests", response_model=JoinRequestRead | None)
def request_to_join(
    group_id: str,
    data: JoinRequestCreate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> JoinRequestRead | None:
    return GroupService(session, user).request_join(group_id, data.message)


@router.get("/{group_id}/join-requests", response_model=list[JoinRequestRead])
def list_join_requests(
    group_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> list[JoinRequestRead]:
    return GroupService(session, user).join_requests(group_id)


@router.patch("/{group_id}/join-requests/{request_id}", response_model=MessageResponse)
def review_join_request(
    group_id: str,
    request_id: str,
    data: JoinRequestReview,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> MessageResponse:
    GroupService(session, user).review_join_request(group_id, request_id, data.decision)
    return MessageResponse(message=f"Join request {data.decision}.")


@router.post("/{group_id}/transfer-ownership", response_model=MessageResponse)
def transfer_ownership(
    group_id: str,
    data: TransferOwnershipRequest,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> MessageResponse:
    GroupService(session, user).transfer_ownership(group_id, data.new_owner_user_id)
    return MessageResponse(message="Ownership transferred.")


@router.post("/{group_id}/leave", response_model=MessageResponse)
def leave_group(
    group_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> MessageResponse:
    GroupService(session, user).leave(group_id)
    return MessageResponse(message="You left the group.")
