from datetime import timedelta
from math import ceil

from sqlalchemy import select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.db.base import utc_now
from app.models import Group, GroupInvitation, GroupJoinRequest, GroupMember, User
from app.repositories.groups import GroupRepository
from app.repositories.users import UserRepository
from app.schemas.common import Page
from app.schemas.group import (
    GroupCreate,
    GroupRead,
    GroupUpdate,
    InvitationRead,
    JoinRequestRead,
    MemberRead,
)
from app.schemas.user import UserSummary
from app.services.audit import write_audit


class GroupService:
    def __init__(self, session: Session, user: User) -> None:
        self.session = session
        self.user = user
        self.groups = GroupRepository(session)
        self.users = UserRepository(session)

    def list_page(
        self,
        *,
        page: int,
        page_size: int,
        managed_only: bool,
    ) -> Page[GroupRead]:
        rows, total = self.groups.list_for_user(
            self.user.id,
            page=page,
            page_size=page_size,
            managed_only=managed_only,
        )
        return Page[GroupRead](
            items=[self._read(group, role) for group, role in rows],
            page=page,
            page_size=page_size,
            total=total,
            pages=ceil(total / page_size) if total else 0,
        )

    def create(self, data: GroupCreate) -> GroupRead:
        group = Group(
            name=data.name.strip(),
            description=data.description,
            group_type="personal",
            owner_id=self.user.id,
            join_policy=data.join_policy,
            allow_member_invite=data.allow_member_invite,
            status="active",
            created_by=self.user.id,
        )
        self.session.add(group)
        self.session.flush()
        self.session.add(
            GroupMember(
                group_id=group.id,
                user_id=self.user.id,
                role="owner",
                status="active",
            )
        )
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="group.created",
            resource_type="group",
            resource_id=group.id,
            after_data={"name": group.name, "join_policy": group.join_policy},
        )
        self.session.commit()
        return self._read(group, "owner")

    def get(self, group_id: str) -> GroupRead:
        group, membership = self._member_context(group_id)
        return self._read(group, membership.role)

    def update(self, group_id: str, data: GroupUpdate) -> GroupRead:
        group, membership = self._manager_context(group_id)
        updates = data.model_dump(exclude_unset=True)
        if membership.role != "owner":
            updates.pop("join_policy", None)
            updates.pop("allow_member_invite", None)
        before = {
            "name": group.name,
            "join_policy": group.join_policy,
            "allow_member_invite": group.allow_member_invite,
        }
        for key, value in updates.items():
            if value is not None:
                setattr(group, key, value)
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="group.updated",
            resource_type="group",
            resource_id=group.id,
            before_data=before,
            after_data=updates,
        )
        self.session.commit()
        return self._read(group, membership.role)

    def members(self, group_id: str) -> list[MemberRead]:
        group, _ = self._member_context(group_id)
        memberships = self.session.scalars(
            select(GroupMember)
            .where(GroupMember.group_id == group.id, GroupMember.status == "active")
            .order_by(GroupMember.role, GroupMember.joined_at)
        )
        return [self._member_read(member) for member in memberships]

    def invite(self, group_id: str, identity: str) -> InvitationRead:
        group, membership = self._member_context(group_id)
        if membership.role not in {"owner", "admin"} and not group.allow_member_invite:
            raise AppError("PERMISSION_DENIED", "You cannot invite members.", 403)
        invitee = self.users.by_username_or_email(identity)
        if invitee is None or invitee.status != "active":
            raise AppError("USER_NOT_FOUND", "An active user was not found.", 404)
        if self.groups.membership(group.id, invitee.id):
            raise AppError("ALREADY_MEMBER", "This user is already a member.", 409)
        existing = self.session.scalar(
            select(GroupInvitation).where(
                GroupInvitation.group_id == group.id,
                GroupInvitation.invitee_id == invitee.id,
                GroupInvitation.status == "pending",
            )
        )
        if existing:
            raise AppError("INVITATION_EXISTS", "A pending invitation already exists.", 409)
        invitation = GroupInvitation(
            group_id=group.id,
            invitee_id=invitee.id,
            invited_by=self.user.id,
            status="pending",
            expires_at=utc_now() + timedelta(days=7),
        )
        self.session.add(invitation)
        self.session.flush()
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="group.member_invited",
            resource_type="group",
            resource_id=group.id,
            after_data={"invitee_id": invitee.id},
        )
        self.session.commit()
        result = InvitationRead.model_validate(invitation)
        result.group_name = group.name
        return result

    def my_invitations(self) -> list[InvitationRead]:
        rows = self.session.execute(
            select(GroupInvitation, Group.name)
            .join(Group, Group.id == GroupInvitation.group_id)
            .where(
                GroupInvitation.invitee_id == self.user.id,
                GroupInvitation.status == "pending",
                Group.status == "active",
            )
            .order_by(GroupInvitation.created_at.desc())
        )
        results: list[InvitationRead] = []
        for invitation, group_name in rows:
            result = InvitationRead.model_validate(invitation)
            result.group_name = group_name
            results.append(result)
        return results

    def respond_invitation(self, invitation_id: str, accept: bool) -> None:
        invitation = self.session.scalar(
            select(GroupInvitation).where(
                GroupInvitation.id == invitation_id,
                GroupInvitation.invitee_id == self.user.id,
                GroupInvitation.status == "pending",
            )
        )
        if invitation is None:
            raise AppError("INVITATION_NOT_FOUND", "Invitation was not found.", 404)
        expires_at = invitation.expires_at.replace(
            tzinfo=invitation.expires_at.tzinfo or utc_now().tzinfo
        )
        if expires_at < utc_now():
            invitation.status = "expired"
            self.session.commit()
            raise AppError("INVITATION_EXPIRED", "Invitation has expired.", 410)
        invitation.status = "accepted" if accept else "declined"
        invitation.responded_at = utc_now()
        if accept:
            self._activate_member(
                invitation.group_id,
                self.user.id,
                invited_by=invitation.invited_by,
            )
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="group.invitation_accepted" if accept else "group.invitation_declined",
            resource_type="group",
            resource_id=invitation.group_id,
        )
        self.session.commit()

    def request_join(self, group_id: str, message: str) -> JoinRequestRead | None:
        group = self.groups.get_active(group_id)
        if group is None:
            raise AppError("GROUP_NOT_FOUND", "Group was not found.", 404)
        if self.groups.membership(group.id, self.user.id):
            raise AppError("ALREADY_MEMBER", "You are already a member.", 409)
        if group.join_policy == "public":
            self._activate_member(group.id, self.user.id)
            write_audit(
                self.session,
                actor_user_id=self.user.id,
                action="group.member_joined",
                resource_type="group",
                resource_id=group.id,
            )
            self.session.commit()
            return None
        if group.join_policy != "approval_required":
            raise AppError("JOIN_NOT_ALLOWED", "This group does not accept join requests.", 403)
        existing = self.session.scalar(
            select(GroupJoinRequest).where(
                GroupJoinRequest.group_id == group.id,
                GroupJoinRequest.user_id == self.user.id,
                GroupJoinRequest.status == "pending",
            )
        )
        if existing:
            raise AppError("JOIN_REQUEST_EXISTS", "A pending request already exists.", 409)
        request = GroupJoinRequest(
            group_id=group.id,
            user_id=self.user.id,
            message=message,
            status="pending",
        )
        self.session.add(request)
        self.session.flush()
        self.session.commit()
        return self._join_request_read(request)

    def join_requests(self, group_id: str) -> list[JoinRequestRead]:
        group, _ = self._manager_context(group_id)
        requests = self.session.scalars(
            select(GroupJoinRequest)
            .where(GroupJoinRequest.group_id == group.id)
            .order_by(GroupJoinRequest.created_at.desc())
        )
        return [self._join_request_read(item) for item in requests]

    def review_join_request(self, group_id: str, request_id: str, decision: str) -> None:
        group, _ = self._manager_context(group_id)
        request = self.session.scalar(
            select(GroupJoinRequest).where(
                GroupJoinRequest.id == request_id,
                GroupJoinRequest.group_id == group.id,
                GroupJoinRequest.status == "pending",
            )
        )
        if request is None:
            raise AppError("JOIN_REQUEST_NOT_FOUND", "Join request was not found.", 404)
        request.status = decision
        request.reviewed_by = self.user.id
        request.reviewed_at = utc_now()
        if decision == "approved":
            self._activate_member(group.id, request.user_id, invited_by=self.user.id)
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action=f"group.join_request_{decision}",
            resource_type="group",
            resource_id=group.id,
            after_data={"user_id": request.user_id},
        )
        self.session.commit()

    def set_role(self, group_id: str, user_id: str, role: str) -> MemberRead:
        group, _ = self._owner_context(group_id)
        target = self.groups.membership(group.id, user_id)
        if target is None or target.role == "owner":
            raise AppError("MEMBER_NOT_FOUND", "Member was not found.", 404)
        before_role = target.role
        target.role = role
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="group.admin_assigned" if role == "admin" else "group.admin_revoked",
            resource_type="group",
            resource_id=group.id,
            before_data={"user_id": user_id, "role": before_role},
            after_data={"user_id": user_id, "role": role},
        )
        self.session.commit()
        return self._member_read(target)

    def remove_member(self, group_id: str, user_id: str) -> None:
        group, actor = self._manager_context(group_id)
        target = self.groups.membership(group.id, user_id)
        if target is None:
            raise AppError("MEMBER_NOT_FOUND", "Member was not found.", 404)
        if target.role == "owner" or (actor.role == "admin" and target.role != "member"):
            raise AppError("PERMISSION_DENIED", "You cannot remove this member.", 403)
        target.status = "removed"
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="group.member_removed",
            resource_type="group",
            resource_id=group.id,
            after_data={"user_id": user_id},
        )
        self.session.commit()

    def transfer_ownership(self, group_id: str, new_owner_user_id: str) -> None:
        group, owner_membership = self._owner_context(group_id)
        target = self.groups.membership(group.id, new_owner_user_id)
        if target is None:
            raise AppError("MEMBER_NOT_FOUND", "New owner must be an active member.", 404)
        target.role = "owner"
        owner_membership.role = "admin"
        group.owner_id = target.user_id
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="group.ownership_transferred",
            resource_type="group",
            resource_id=group.id,
            before_data={"owner_id": self.user.id},
            after_data={"owner_id": target.user_id},
        )
        self.session.commit()

    def leave(self, group_id: str) -> None:
        group, membership = self._member_context(group_id)
        if membership.role == "owner":
            raise AppError(
                "OWNER_CANNOT_LEAVE",
                "Transfer ownership or dissolve the group before leaving.",
                409,
            )
        membership.status = "left"
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="group.member_left",
            resource_type="group",
            resource_id=group.id,
        )
        self.session.commit()

    def dissolve(self, group_id: str) -> None:
        group, _ = self._owner_context(group_id)
        group.status = "deleted"
        group.deleted_at = utc_now()
        write_audit(
            self.session,
            actor_user_id=self.user.id,
            action="group.dissolved",
            resource_type="group",
            resource_id=group.id,
            before_data={"name": group.name},
        )
        self.session.commit()

    def _member_context(self, group_id: str) -> tuple[Group, GroupMember]:
        group = self.groups.get_active(group_id)
        membership = self.groups.membership(group_id, self.user.id) if group else None
        if group is None or membership is None:
            raise AppError("GROUP_NOT_FOUND", "Group was not found.", 404)
        return group, membership

    def _manager_context(self, group_id: str) -> tuple[Group, GroupMember]:
        group, membership = self._member_context(group_id)
        if membership.role not in {"owner", "admin"}:
            raise AppError("PERMISSION_DENIED", "Group administrator permission is required.", 403)
        return group, membership

    def _owner_context(self, group_id: str) -> tuple[Group, GroupMember]:
        group, membership = self._member_context(group_id)
        if membership.role != "owner":
            raise AppError("PERMISSION_DENIED", "Group owner permission is required.", 403)
        return group, membership

    def _read(self, group: Group, role: str) -> GroupRead:
        result = GroupRead.model_validate(group)
        result.current_user_role = role
        return result

    def _member_read(self, membership: GroupMember) -> MemberRead:
        result = MemberRead.model_validate(membership)
        user = self.users.get(membership.user_id)
        result.user = UserSummary.model_validate(user) if user else None
        return result

    def _join_request_read(self, request: GroupJoinRequest) -> JoinRequestRead:
        result = JoinRequestRead.model_validate(request)
        user = self.users.get(request.user_id)
        result.user = UserSummary.model_validate(user) if user else None
        return result

    def _activate_member(
        self,
        group_id: str,
        user_id: str,
        *,
        invited_by: str | None = None,
    ) -> GroupMember:
        membership = self.session.scalar(
            select(GroupMember).where(
                GroupMember.group_id == group_id,
                GroupMember.user_id == user_id,
            )
        )
        if membership is None:
            membership = GroupMember(
                group_id=group_id,
                user_id=user_id,
                role="member",
                status="active",
                invited_by=invited_by,
            )
            self.session.add(membership)
        else:
            membership.role = "member"
            membership.status = "active"
            membership.joined_at = utc_now()
            membership.invited_by = invited_by
        return membership
