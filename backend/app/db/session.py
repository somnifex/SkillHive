from collections.abc import Generator
from pathlib import Path

from sqlalchemy import Engine, create_engine, event, text
from sqlalchemy.orm import Session, sessionmaker

from app.core.config import settings


def _ensure_sqlite_parent(database_url: str) -> None:
    prefix = "sqlite:///"
    if not database_url.startswith(prefix) or database_url.endswith(":memory:"):
        return
    path = Path(database_url.removeprefix(prefix))
    path.parent.mkdir(parents=True, exist_ok=True)


def make_engine(database_url: str) -> Engine:
    _ensure_sqlite_parent(database_url)
    connect_args = {"check_same_thread": False} if database_url.startswith("sqlite") else {}
    engine = create_engine(database_url, pool_pre_ping=True, connect_args=connect_args)
    if database_url.startswith("sqlite"):
        event.listen(engine, "connect", _enable_sqlite_foreign_keys)
    return engine


def _enable_sqlite_foreign_keys(dbapi_connection: object, _connection_record: object) -> None:
    cursor = dbapi_connection.cursor()  # type: ignore[attr-defined]
    cursor.execute("PRAGMA foreign_keys=ON")
    cursor.close()


def begin_sqlite_immediate_write(session: Session) -> None:
    """Start the SQLite write transaction used by read/modify/flush paths.

    SQLite does not implement row-level ``FOR UPDATE`` locks.  An immediate
    transaction takes the database writer lock before the caller reads the
    revision head, making optimistic-concurrency validation atomic with the
    subsequent mutation.  Other dialects keep their normal transaction
    behavior and rely on row locks where supported.
    """
    if session.get_bind().dialect.name == "sqlite":
        session.rollback()
        session.execute(text("BEGIN IMMEDIATE"))


engine = make_engine(settings.database_url)
SessionLocal = sessionmaker(bind=engine, autoflush=False, expire_on_commit=False)


def get_db() -> Generator[Session, None, None]:
    with SessionLocal() as session:
        yield session
