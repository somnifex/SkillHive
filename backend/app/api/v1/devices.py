"""M2.3 device identity endpoints.

Device registration is idempotent per ``(user, client_instance_id)``.
Revocation is server-authoritative; sync endpoints validate device state at
execution time so a revoked device loses authorization without pending local
mutations being destroyed.
"""

from typing import Annotated

from fastapi import APIRouter, Depends
from sqlalchemy import select
from sqlalchemy.orm import Session

from app.core.exceptions import AppError
from app.db.base import utc_now
from app.db.session import get_db
from app.models import Device
from app.permissions.dependencies import CurrentUser
from app.schemas.sync import DeviceRead, DeviceRegisterRequest

router = APIRouter(prefix="/devices", tags=["devices"])

DBSession = Annotated[Session, Depends(get_db)]


@router.post("/register", response_model=DeviceRead, status_code=201)
def register_device(
    request: DeviceRegisterRequest,
    user: CurrentUser,
    session: DBSession,
) -> DeviceRead:
    """Register the calling installation; idempotent per (user, clientInstance)."""
    client_instance_id = str(request.client_instance_id)
    existing = session.execute(
        select(Device).where(
            Device.user_id == user.id,
            Device.client_instance_id == client_instance_id,
        )
    ).scalar_one_or_none()
    if existing is not None:
        if existing.revoked_at is not None:
            raise AppError(
                "DEVICE_REVOKED",
                "This device registration has been revoked; re-registration is not allowed.",
                403,
            )
        existing.display_name = request.display_name or existing.display_name
        existing.platform = request.platform or existing.platform
        existing.app_version = request.app_version or existing.app_version
        existing.last_seen_at = utc_now()
        session.commit()
        session.refresh(existing)
        return _read_device(existing)

    device = Device(
        user_id=user.id,
        client_instance_id=client_instance_id,
        display_name=request.display_name,
        platform=request.platform,
        app_version=request.app_version,
        last_seen_at=utc_now(),
    )
    session.add(device)
    session.commit()
    session.refresh(device)
    return _read_device(device)


@router.get("", response_model=list[DeviceRead])
def list_devices(user: CurrentUser, session: DBSession) -> list[DeviceRead]:
    devices = (
        session.execute(
            select(Device)
            .where(Device.user_id == user.id)
            .order_by(Device.created_at.asc(), Device.id.asc())
        )
        .scalars()
        .all()
    )
    return [_read_device(device) for device in devices]


@router.delete("/{device_id}", status_code=204)
def revoke_device(device_id: str, user: CurrentUser, session: DBSession) -> None:
    device = session.execute(
        select(Device).where(Device.id == device_id, Device.user_id == user.id)
    ).scalar_one_or_none()
    if device is None:
        raise AppError("DEVICE_NOT_FOUND", "Device was not found.", 404)
    if device.revoked_at is None:
        device.revoked_at = utc_now()
        session.commit()


def _read_device(device: Device) -> DeviceRead:
    return DeviceRead(
        device_id=device.id,
        client_instance_id=device.client_instance_id,
        display_name=device.display_name,
        platform=device.platform,
        app_version=device.app_version,
        last_seen_at=device.last_seen_at,
        revoked_at=device.revoked_at,
    )
