from typing import Literal

from pydantic import BaseModel, Field


class SystemSettingsRead(BaseModel):
    blob_storage_backend: Literal["local", "s3"]
    s3_endpoint_url: str | None
    s3_bucket: str | None
    s3_prefix: str
    s3_region: str | None
    # Credentials themselves live only in server environment variables; this
    # flag tells the console whether they are present.
    s3_credentials_configured: bool
    allow_registration: bool
    max_package_bytes: int | None


class SystemSettingsUpdate(BaseModel):
    blob_storage_backend: Literal["local", "s3"] | None = None
    s3_endpoint_url: str | None = Field(default=None, max_length=500)
    s3_bucket: str | None = Field(default=None, max_length=255)
    s3_prefix: str | None = Field(default=None, max_length=255)
    s3_region: str | None = Field(default=None, max_length=64)
    allow_registration: bool | None = None
    max_package_bytes: int | None = Field(default=None, ge=1)


class AdminUserCreate(BaseModel):
    username: str = Field(min_length=3, max_length=50)
    display_name: str = Field(min_length=1, max_length=100)
    email: str = Field(max_length=255)
    password: str = Field(min_length=8, max_length=128)
    is_global_admin: bool = False


class AdminPasswordReset(BaseModel):
    new_password: str = Field(min_length=8, max_length=128)
