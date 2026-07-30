from typing import Annotated, Literal

from fastapi import APIRouter, Depends, Query, Response
from sqlalchemy.orm import Session

from app.db.session import get_db
from app.permissions.dependencies import CurrentUser
from app.schemas.common import Page
from app.schemas.skill import (
    SkillCreate,
    SkillRead,
    SkillUpdate,
    SkillVersionCreate,
    SkillVersionRead,
)
from app.services.skills import PrivateSkillService

router = APIRouter(prefix="/skills", tags=["private skills"])


@router.get("", response_model=Page[SkillRead])
def list_skills(
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
    page: Annotated[int, Query(ge=1)] = 1,
    page_size: Annotated[int, Query(ge=1, le=100)] = 20,
    query: Annotated[str | None, Query(max_length=120)] = None,
    category: Annotated[str | None, Query(max_length=80)] = None,
    tag: Annotated[str | None, Query(max_length=50)] = None,
    status: Annotated[str | None, Query()] = None,
    sort: Literal["name", "created_at", "updated_at"] = "updated_at",
    order: Literal["asc", "desc"] = "desc",
) -> Page[SkillRead]:
    return PrivateSkillService(session, user).list_page(
        page=page,
        page_size=page_size,
        query=query,
        category=category,
        tag=tag,
        status=status,
        sort=sort,
        order=order,
    )


@router.post("", response_model=SkillRead, status_code=201)
def create_skill(
    data: SkillCreate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return PrivateSkillService(session, user).create(data)


@router.get("/{skill_id}", response_model=SkillRead)
def get_skill(
    skill_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return PrivateSkillService(session, user).get(skill_id)


@router.patch("/{skill_id}", response_model=SkillRead)
def update_skill(
    skill_id: str,
    data: SkillUpdate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return PrivateSkillService(session, user).update(skill_id, data)


@router.delete("/{skill_id}", status_code=204)
def delete_skill(
    skill_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> Response:
    PrivateSkillService(session, user).delete(skill_id)
    return Response(status_code=204)


@router.post("/{skill_id}/copy", response_model=SkillRead, status_code=201)
def copy_skill(
    skill_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return PrivateSkillService(session, user).copy(skill_id)


@router.get("/{skill_id}/versions", response_model=list[SkillVersionRead])
def list_versions(
    skill_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> list[SkillVersionRead]:
    return PrivateSkillService(session, user).versions(skill_id)


@router.post("/{skill_id}/versions", response_model=SkillVersionRead, status_code=201)
def create_version(
    skill_id: str,
    data: SkillVersionCreate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillVersionRead:
    return PrivateSkillService(session, user).create_version(skill_id, data)
