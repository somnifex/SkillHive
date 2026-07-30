from typing import Annotated, Literal

from fastapi import APIRouter, Depends, Query, Response
from sqlalchemy.orm import Session

from app.db.session import get_db
from app.permissions.dependencies import CurrentUser
from app.schemas.common import Page
from app.schemas.skill import SkillRead
from app.schemas.template import (
    TemplateCreate,
    TemplateInstantiate,
    TemplateRead,
    TemplateUpdate,
)
from app.services.templates import TemplateService

router = APIRouter(prefix="/templates", tags=["skill templates"])


@router.get("", response_model=Page[TemplateRead])
def list_templates(
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
    page: Annotated[int, Query(ge=1)] = 1,
    page_size: Annotated[int, Query(ge=1, le=100)] = 20,
    query: Annotated[str | None, Query(max_length=120)] = None,
    scope_type: Literal["personal", "group", "global"] | None = None,
    group_id: str | None = None,
) -> Page[TemplateRead]:
    return TemplateService(session, user).list_page(
        page=page,
        page_size=page_size,
        query=query,
        scope_type=scope_type,
        group_id=group_id,
    )


@router.post("", response_model=TemplateRead, status_code=201)
def create_template(
    data: TemplateCreate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> TemplateRead:
    return TemplateService(session, user).create(data)


@router.get("/{template_id}", response_model=TemplateRead)
def get_template(
    template_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> TemplateRead:
    return TemplateService(session, user).get(template_id)


@router.patch("/{template_id}", response_model=TemplateRead)
def update_template(
    template_id: str,
    data: TemplateUpdate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> TemplateRead:
    return TemplateService(session, user).update(template_id, data)


@router.delete("/{template_id}", status_code=204)
def delete_template(
    template_id: str,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> Response:
    TemplateService(session, user).delete(template_id)
    return Response(status_code=204)


@router.post("/{template_id}/instantiate", response_model=SkillRead, status_code=201)
def instantiate_template(
    template_id: str,
    data: TemplateInstantiate,
    user: CurrentUser,
    session: Annotated[Session, Depends(get_db)],
) -> SkillRead:
    return TemplateService(session, user).instantiate(template_id, data)
