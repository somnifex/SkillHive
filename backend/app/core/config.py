from functools import lru_cache

from pydantic import Field
from pydantic_settings import BaseSettings, SettingsConfigDict


class Settings(BaseSettings):
    model_config = SettingsConfigDict(
        env_file=".env",
        env_file_encoding="utf-8",
        case_sensitive=False,
        extra="ignore",
    )

    app_name: str = "SkillHive"
    environment: str = "development"
    debug: bool = False
    log_level: str = "INFO"
    api_v1_prefix: str = "/api/v1"
    database_url: str = "sqlite:///./data/skillhive.db"
    blob_storage_path: str = "./data/blobs"
    jwt_secret_key: str = Field(
        default="development-only-change-me-at-least-32-bytes",
        min_length=16,
    )
    jwt_algorithm: str = "HS256"
    access_token_expire_minutes: int = 15
    refresh_token_expire_days: int = 7
    cors_origins: list[str] = [
        "http://127.0.0.1:5173",
        "http://localhost:5173",
    ]
    cookie_secure: bool = False
    login_max_attempts: int = 5
    login_lockout_minutes: int = 15

    # Blob/change-log GC (docs/development/GC_DESIGN.md §8). All defaults are
    # the conservative values from the design; the sweep only collects
    # objects outside every mark root and past the orphan grace.
    blob_gc_orphan_grace_hours: int = 24
    change_log_retention_days: int = 90
    receipt_retention_days: int = 90
    blob_gc_batch_size: int = 1000


@lru_cache
def get_settings() -> Settings:
    return Settings()


settings = get_settings()
