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
    # Storage backend selection (system admin console / design doc §4). The
    # DB-stored system setting overrides this default; credentials always stay
    # in environment variables and are never persisted to the database.
    blob_storage_backend: str = "local"
    s3_endpoint_url: str | None = None
    s3_bucket: str | None = None
    s3_prefix: str = ""
    s3_region: str | None = None
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
        # Tauri 2 WebView origins (Windows uses http, macOS/Linux tauri://);
        # without these the desktop login preflight is rejected.
        "http://tauri.localhost",
        "https://tauri.localhost",
        "tauri://localhost",
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
    # One bounded maintenance pass per interval.  The worker runs trash
    # retention, change/receipt trim, then blob GC sequentially so those
    # destructive operations cannot race each other in one process.
    maintenance_interval_seconds: int = 24 * 60 * 60


@lru_cache
def get_settings() -> Settings:
    return Settings()


settings = get_settings()
