"""Admin-editable platform settings (storage backend, registration, limits).

The active values live in the ``system_settings`` table under the single
``platform`` key and override the environment defaults. Secrets are never
stored here: S3 credentials come from server environment variables and the
settings only record whether they are configured.
"""

from typing import Any

from sqlalchemy.orm import Session

from app.core.config import settings
from app.core.exceptions import AppError
from app.models import SystemSetting, User
from app.schemas.system_settings import SystemSettingsRead, SystemSettingsUpdate
from app.services.audit import write_audit
from app.services.blob_storage import (
    SETTINGS_KEY,
    effective_storage_config,
    s3_credentials_present,
)


def registration_enabled(session: Session) -> bool:
    row = session.get(SystemSetting, SETTINGS_KEY)
    value = row.value if row is not None and isinstance(row.value, dict) else {}
    return bool(value.get("allow_registration", True))


class SystemSettingsService:
    def __init__(self, session: Session, admin: User | None = None) -> None:
        self.session = session
        self.admin = admin

    def read(self) -> SystemSettingsRead:
        config = effective_storage_config(self.session)
        value = self._stored_value()
        return SystemSettingsRead(
            blob_storage_backend="s3" if config["backend"] == "s3" else "local",
            s3_endpoint_url=config["s3_endpoint_url"],
            s3_bucket=config["s3_bucket"],
            s3_prefix=config["s3_prefix"] or "",
            s3_region=config["s3_region"],
            s3_credentials_configured=s3_credentials_present(),
            allow_registration=bool(value.get("allow_registration", True)),
            max_package_bytes=value.get("max_package_bytes"),
        )

    def update(self, data: SystemSettingsUpdate) -> SystemSettingsRead:
        row = self.session.get(SystemSetting, SETTINGS_KEY)
        value = self._stored_value()
        updates = data.model_dump(exclude_unset=True)
        if updates.get("blob_storage_backend") == "s3":
            self._validate_s3_ready(updates, value)
        for key, val in updates.items():
            if val is None:
                # An explicit null resets the entry to the environment default.
                value.pop(key, None)
            else:
                value[key] = val
        before = dict(row.value) if row is not None and isinstance(row.value, dict) else {}
        if row is None:
            row = SystemSetting(
                key=SETTINGS_KEY,
                value=value,
                updated_by=self.admin.id if self.admin else None,
            )
            self.session.add(row)
        else:
            row.value = value
            row.updated_by = self.admin.id if self.admin else None
        write_audit(
            self.session,
            actor_user_id=self.admin.id if self.admin else None,
            action="system.settings_updated",
            resource_type="system",
            resource_id=SETTINGS_KEY,
            before_data=before,
            after_data=value,
        )
        self.session.commit()
        return self.read()

    def _stored_value(self) -> dict[str, Any]:
        row = self.session.get(SystemSetting, SETTINGS_KEY)
        return dict(row.value) if row is not None and isinstance(row.value, dict) else {}

    def _validate_s3_ready(
        self,
        updates: dict[str, Any],
        stored: dict[str, Any],
    ) -> None:
        if not s3_credentials_present():
            raise AppError(
                "S3_CREDENTIALS_MISSING",
                "S3 credentials must be configured in the server environment "
                "(S3_ACCESS_KEY_ID / S3_SECRET_ACCESS_KEY) before switching backends.",
                409,
            )
        bucket = (
            updates.get("s3_bucket")
            or stored.get("s3_bucket")
            or settings.s3_bucket
        )
        if not bucket:
            raise AppError(
                "S3_BUCKET_NOT_CONFIGURED",
                "A bucket must be configured before switching to S3 storage.",
                400,
            )
