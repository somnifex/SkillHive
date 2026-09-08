from typing import Annotated, Literal

from fastapi import APIRouter, Depends, Query, Response
from sqlalchemy.orm import Session

from app.db.session import get_db
from app.permissions.dependencies import CurrentUser
from app.schemas.common import Page
from app.schemas.skill import (
    GroupSkillGrantCreate,
    GroupSkillGrantRead,
    GroupSkillGrantUpdate,
    SkillCreate,
    SkillRead,
    SkillUpdate,
    SkillVersionCreate,
    SkillVersionRead,
    VersionRollbackRequest,
    VersionTagsUpdate,
)
from app.services.group_owned_skills import GroupOwnedSkillService
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


@router.get("/shared", response_model=Page[SkillRead])
def list_group_owned_skills(
    group_id: str,
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
    return GroupOwnedSkillService(session, user).list_page(
        group_id,
        page=page,
        page_size=page_size,
        query=query,
        category=category,
        tag=tag,
        status=status,
        sort=sort,
        order=order,
    )


@router.post("/shared", response_model=SkillRead, status_code=201)
def create_group_owned_skill(
    group_id: str,
    data: SkillCreate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return GroupOwnedSkillService(session, user).create(group_id, data)


@router.get("/shared/trash", response_model=Page[SkillRead])
def list_group_owned_skill_trash(
    group_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
    page: Annotated[int, Query(ge=1)] = 1,
    page_size: Annotated[int, Query(ge=1, le=100)] = 50,
    query: Annotated[str | None, Query(max_length=120)] = None,
) -> Page[SkillRead]:
    return GroupOwnedSkillService(session, user).trash_page(
        group_id,
        page=page,
        page_size=page_size,
        query=query,
    )


@router.get("/shared/{skill_id}", response_model=SkillRead)
def get_group_owned_skill(
    group_id: str,
    skill_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return GroupOwnedSkillService(session, user).get(group_id, skill_id)


@router.patch("/shared/{skill_id}", response_model=SkillRead)
def update_group_owned_skill(
    group_id: str,
    skill_id: str,
    data: SkillUpdate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return GroupOwnedSkillService(session, user).update(group_id, skill_id, data)


@router.delete("/shared/{skill_id}", status_code=204)
def delete_group_owned_skill(
    group_id: str,
    skill_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> Response:
    GroupOwnedSkillService(session, user).delete(group_id, skill_id)
    return Response(status_code=204)


@router.post("/shared/{skill_id}/restore", response_model=SkillRead)
def restore_group_owned_skill(
    group_id: str,
    skill_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return GroupOwnedSkillService(session, user).restore(group_id, skill_id)


@router.delete("/shared/{skill_id}/purge", status_code=204)
def purge_group_owned_skill(
    group_id: str,
    skill_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> Response:
    GroupOwnedSkillService(session, user).purge(group_id, skill_id)
    return Response(status_code=204)


@router.get("/shared/{skill_id}/versions", response_model=list[SkillVersionRead])
def list_group_skill_versions(
    group_id: str,
    skill_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> list[SkillVersionRead]:
    return GroupOwnedSkillService(session, user).versions(group_id, skill_id)


@router.post("/shared/{skill_id}/versions", response_model=SkillVersionRead, status_code=201)
def create_group_skill_version(
    group_id: str,
    skill_id: str,
    data: SkillVersionCreate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillVersionRead:
    return GroupOwnedSkillService(session, user).create_version(group_id, skill_id, data)


@router.put(
    "/shared/{skill_id}/versions/{version}/tags",
    response_model=SkillVersionRead,
)
def set_group_skill_version_tags(
    group_id: str,
    skill_id: str,
    version: str,
    data: VersionTagsUpdate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillVersionRead:
    return GroupOwnedSkillService(session, user).set_version_tags(
        group_id, skill_id, version, data.tags
    )


@router.post("/shared/{skill_id}/rollback", response_model=SkillVersionRead, status_code=201)
def rollback_group_skill_version(
    group_id: str,
    skill_id: str,
    data: VersionRollbackRequest,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillVersionRead:
    return GroupOwnedSkillService(session, user).rollback(group_id, skill_id, data)


@router.get("/shared/{skill_id}/versions/{version}/export")
def export_group_skill_version(
    group_id: str,
    skill_id: str,
    version: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> Response:
    payload = GroupOwnedSkillService(session, user).export_version_zip(
        group_id, skill_id, version
    )
    filename = f"{version.replace('.', '_')}.zip"
    return Response(
        content=payload,
        media_type="application/zip",
        headers={"Content-Disposition": f'attachment; filename="{filename}"'},
    )


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
