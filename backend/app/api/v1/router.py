from fastapi import APIRouter

from app.api.v1.admin import router as admin_router
from app.api.v1.auth import router as auth_router
from app.api.v1.devices import router as devices_router
from app.api.v1.group_skills import router as group_skills_router
from app.api.v1.groups import router as groups_router
from app.api.v1.skills import router as skills_router
from app.api.v1.sync import router as sync_router
from app.api.v1.templates import router as templates_router

router = APIRouter()
router.include_router(admin_router)
router.include_router(auth_router)
router.include_router(devices_router)
router.include_router(group_skills_router)
router.include_router(groups_router)
router.include_router(skills_router)
router.include_router(templates_router)
router.include_router(sync_router)


@router.get("/health", tags=["system"], summary="Health check")
def health_check() -> dict[str, str]:
    return {"status": "ok", "service": "skillhive-api"}
