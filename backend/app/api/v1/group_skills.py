from typing import Annotated

from fastapi import APIRouter, Depends, Response
from sqlalchemy.orm import Session

from app.db.session import get_db
from app.permissions.dependencies import CurrentUser
from app.schemas.skill import (
    GroupSkillGrantCreate,
    GroupSkillGrantRead,
    GroupSkillGrantUpdate,
    SkillRead,
    SkillVersionRead,
)
from app.services.group_skills import GroupSkillService

router = APIRouter(prefix="/groups/{group_id}/skills", tags=["group skills"])


@router.get("", response_model=list[GroupSkillGrantRead])
def enabled_group_skills(
    group_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> list[GroupSkillGrantRead]:
    return GroupSkillService(session, user).enabled(group_id)


@router.get("/catalog", response_model=list[SkillRead])
def global_skill_catalog(
    group_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> list[SkillRead]:
    return GroupSkillService(session, user).catalog(group_id)


@router.get("/catalog/{skill_id}/versions", response_model=list[SkillVersionRead])
def global_skill_catalog_versions(
    group_id: str,
    skill_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> list[SkillVersionRead]:
    return GroupSkillService(session, user).catalog_versions(group_id, skill_id)


@router.post("/{skill_id}", response_model=GroupSkillGrantRead, status_code=201)
def enable_group_skill(
    group_id: str,
    skill_id: str,
    data: GroupSkillGrantCreate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> GroupSkillGrantRead:
    return GroupSkillService(session, user).grant(group_id, skill_id, data)


@router.patch("/{skill_id}", response_model=GroupSkillGrantRead)
def update_group_skill(
    group_id: str,
    skill_id: str,
    data: GroupSkillGrantUpdate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> GroupSkillGrantRead:
    return GroupSkillService(session, user).update(group_id, skill_id, data)


@router.delete("/{skill_id}", status_code=204)
def disable_group_skill(
    group_id: str,
    skill_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> Response:
    GroupSkillService(session, user).disable(group_id, skill_id)
    return Response(status_code=204)
