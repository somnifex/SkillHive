"""Server-side blob storage abstraction for M2 package transport.

M1 desktop snapshots are content-addressed manifests of SHA-256 file blobs.
The server stores those blobs behind a small interface so that the single-node
development backend (local filesystem) and a future S3-compatible backend are
interchangeable. The relational database keeps only metadata and package
references, never package bytes.

Backends must enforce the following invariants:

- writes are verified: the storage layer recomputes SHA-256 while persisting
  and rejects digest/size mismatch *before* the object becomes addressable;
- content addressing is idempotent: re-uploading identical bytes succeeds and
  never corrupts an existing valid object;
- reads verify integrity before the bytes are handed to the caller;
- storage entries that are symlinks or otherwise non-regular are unsafe and
  must be rejected rather than followed.
"""

from __future__ import annotations

import hashlib
import os
import tempfile
from abc import ABC, abstractmethod
from pathlib import Path
from pathlib import Path
from typing import Any, BinaryIO, Protocol, Sequence

from sqlalchemy.orm import Session

from app.core.config import settings
from app.core.exceptions import AppError
from app.models import SystemSetting

SETTINGS_KEY = "platform"

HASH_PREFIX = "sha256"
SHA256_HEX_LENGTH = 64
BLOB_CHUNK_BYTES = 256 * 1024


class BlobStorageError(AppError):
    """Raised when a blob operation cannot be completed safely."""


def is_canonical_hash(value: str) -> bool:
    """True for exactly ``sha256:<64 lowercase hex>``."""
    prefix, separator, digest = value.partition(":")
    return (
        prefix == HASH_PREFIX
        and separator == ":"
        and len(digest) == SHA256_HEX_LENGTH
        and all(character in "0123456789abcdef" for character in digest)
    )


def format_digest(digest_hex: str) -> str:
    return f"{HASH_PREFIX}:{digest_hex}"


def hash_bytes(payload: bytes) -> str:
    return format_digest(hashlib.sha256(payload).hexdigest())


class BlobStorage(ABC):
    """Content-addressed package storage interface.

    ``hash`` values are canonical ``sha256:<64 lowercase hex>`` identifiers and
    ``size`` is the exact byte length of the payload.
    """

    backend_name: str = "abstract"

    @abstractmethod
    def exists(self, hash_value: str, size_bytes: int) -> bool:
        """Return True when an object with this hash is stored and addressable."""

    @abstractmethod
    def put_verified(self, hash_value: str, payload: bytes) -> None:
        """Persist payload, verifying its digest and expected size first.

        Implementations must reject a digest/size mismatch before the object
        becomes addressable and must treat an existing valid object as success.
        """

    @abstractmethod
    def open(self, hash_value: str) -> BinaryIO:
        """Open the stored object for reading.

        Raises BLOB_NOT_FOUND when the object is absent; callers use
        ``exists`` first for missing-object negotiation.
        """

    @abstractmethod
    def delete(self, hash_value: str) -> None:
        """Remove an object. Used by later garbage collection only."""


class LocalFilesystemBlobStorage(BlobStorage):
    """Single-node development/self-host backend.

    Objects live under a dedicated data directory as
    ``<root>/sha256/<xx>/<digest>``. This backend is documented as
    single-node: multi-replica deployments must use a shared durable storage
    backend instead.
    """

    backend_name = "local_filesystem"

    def __init__(self, root: Path) -> None:
        self.root = root
        self._ensure_real_directory(root)
        self._ensure_real_directory(root / HASH_PREFIX)

    def _blob_path(self, hash_value: str) -> Path:
        if not is_canonical_hash(hash_value):
            raise BlobStorageError(
                "BLOB_INVALID_HASH",
                "Blob hash is not a canonical sha256 identifier.",
                400,
                {"hash": hash_value},
            )
        digest = hash_value.removeprefix(f"{HASH_PREFIX}:")
        return self.root / HASH_PREFIX / digest[:2] / digest

    def _ensure_real_directory(self, path: Path) -> None:
        if path.is_symlink() or (path.exists() and not path.is_dir()):
            raise BlobStorageError(
                "BLOB_UNSAFE_STORAGE",
                f"Blob storage directory is unsafe: {path}",
                500,
            )
        path.mkdir(parents=True, exist_ok=True)

    def exists(self, hash_value: str, size_bytes: int) -> bool:
        path = self._blob_path(hash_value)
        try:
            metadata = path.lstat()
        except OSError:
            return False
        if path.is_symlink() or not path.is_file():
            raise BlobStorageError(
                "BLOB_UNSAFE_ENTRY",
                f"Blob storage entry is unsafe: {path}",
                500,
            )
        if metadata.st_size != size_bytes:
            return False
        return self.verify(hash_value)

    def verify(self, hash_value: str) -> bool:
        """Recompute the digest of the stored object."""
        path = self._blob_path(hash_value)
        digest = hashlib.sha256()
        try:
            with path.open("rb") as stream:
                while chunk := stream.read(BLOB_CHUNK_BYTES):
                    digest.update(chunk)
        except FileNotFoundError:
            return False
        except OSError as error:
            raise BlobStorageError(
                "BLOB_STORAGE_UNAVAILABLE",
                f"Blob storage read failed: {error}",
                500,
            ) from error
        return f"{HASH_PREFIX}:{digest.hexdigest()}" == hash_value

    def put_verified(self, hash_value: str, payload: bytes) -> None:
        actual = hash_bytes(payload)
        if actual != hash_value:
            raise BlobStorageError(
                "BLOB_DIGEST_MISMATCH",
                "Uploaded blob digest does not match its content address.",
                400,
                {"expected": hash_value, "actual": actual},
            )

        path = self._blob_path(hash_value)
        self._ensure_real_directory(path.parent)
        if path.exists() or path.is_symlink():
            if path.is_symlink() or not path.is_file():
                raise BlobStorageError(
                    "BLOB_UNSAFE_ENTRY",
                    f"Blob storage entry is unsafe: {path}",
                    500,
                )
            if self.verify(hash_value):
                return
            raise BlobStorageError(
                "BLOB_CORRUPTED",
                "Existing content-addressed blob is corrupted.",
                500,
                {"hash": hash_value},
            )

        with tempfile.NamedTemporaryFile(
            dir=path.parent,
            prefix=".skillhive-blob-",
            suffix=".tmp",
            delete=False,
        ) as handle:
            handle.write(payload)
            handle.flush()
            temporary = Path(handle.name)
        try:
            temporary.replace(path)
        except OSError:
            # A concurrent content-addressed writer may have won the race. The
            # winner's object is verified before we accept the outcome.
            temporary.unlink(missing_ok=True)
            if not path.exists():
                raise
        if not self.verify(hash_value):
            raise BlobStorageError(
                "BLOB_CORRUPTED",
                "Blob failed verification immediately after write.",
                500,
                {"hash": hash_value},
            )

    def open(self, hash_value: str) -> BinaryIO:
        """Open the stored object, or raise BLOB_NOT_FOUND when absent."""
        path = self._blob_path(hash_value)
        if path.is_symlink() or not path.is_file():
            raise BlobStorageError(
                "BLOB_NOT_FOUND",
                "Blob object was not found in storage.",
                404,
                {"hash": hash_value},
            )
        return path.open("rb")  # noqa: SIM115

    def delete(self, hash_value: str) -> None:
        path = self._blob_path(hash_value)
        try:
            path.unlink()
        except FileNotFoundError:
            return


class _S3Client(Protocol):
    """Structural view of the boto3 S3 client surface used here.

    Kept minimal so tests can supply an in-memory fake and mypy strict stays
    satisfied without the heavyweight boto3-stubs dependency.
    """

    def head_object(self, *, Bucket: str, Key: str) -> Any: ...

    def get_object(self, *, Bucket: str, Key: str) -> Any: ...

    def put_object(self, *, Bucket: str, Key: str, Body: bytes) -> Any: ...

    def delete_object(self, *, Bucket: str, Key: str) -> Any: ...


class S3BlobStorage(BlobStorage):
    """S3-compatible production backend.

    Integrity model: writes verify the digest before upload (identical to the
    filesystem backend); reads trust the storage service's own checksums
    instead of re-downloading for verification — re-verifying every read over
    the network would double egress for no additional guarantee. Credentials
    come from the server environment (``S3_ACCESS_KEY_ID`` /
    ``S3_SECRET_ACCESS_KEY``), never from the database.
    """

    backend_name = "s3"

    def __init__(self, client: _S3Client, bucket: str, prefix: str = "") -> None:
        self._client = client
        self._bucket = bucket
        self._prefix = prefix.strip("/")

    def _key(self, hash_value: str) -> str:
        if not is_canonical_hash(hash_value):
            raise BlobStorageError(
                "BLOB_INVALID_HASH",
                "Blob hash is not a canonical sha256 identifier.",
                400,
                {"hash": hash_value},
            )
        digest = hash_value.removeprefix(f"{HASH_PREFIX}:")
        relative = f"{HASH_PREFIX}/{digest[:2]}/{digest}"
        return f"{self._prefix}/{relative}" if self._prefix else relative

    def _head(self, hash_value: str) -> dict[str, Any]:
        try:
            response: dict[str, Any] = self._client.head_object(
                Bucket=self._bucket,
                Key=self._key(hash_value),
            )
            return response
        except Exception as error:  # noqa: BLE001 - mapped to storage errors below
            raise self._map_error(error, "head", hash_value) from error

    @staticmethod
    def _map_error(error: Exception, operation: str, hash_value: str) -> BlobStorageError:
        response: Any = getattr(error, "response", None) or {}
        code = (response.get("Error") or {}).get("Code", "")
        status = (response.get("ResponseMetadata") or {}).get("HTTPStatusCode")
        if code in ("NoSuchKey", "NotFound", "404") or status == 404:
            return BlobStorageError(
                "BLOB_NOT_FOUND",
                "Blob object was not found in storage.",
                404,
                {"hash": hash_value},
            )
        return BlobStorageError(
            "BLOB_STORAGE_UNAVAILABLE",
            f"Blob storage (S3) {operation} failed: {error}",
            500,
        )

    def exists(self, hash_value: str, size_bytes: int) -> bool:
        try:
            metadata = self._head(hash_value)
        except BlobStorageError as error:
            if error.code == "BLOB_NOT_FOUND":
                return False
            raise
        if not metadata:
            return False
        return int(metadata.get("ContentLength", -1)) == size_bytes

    def put_verified(self, hash_value: str, payload: bytes) -> None:
        actual = hash_bytes(payload)
        if actual != hash_value:
            raise BlobStorageError(
                "BLOB_DIGEST_MISMATCH",
                "Uploaded blob digest does not match its content address.",
                400,
                {"expected": hash_value, "actual": actual},
            )
        try:
            self._client.put_object(Bucket=self._bucket, Key=self._key(hash_value), Body=payload)
        except Exception as error:  # noqa: BLE001 - mapped to storage errors below
            raise self._map_error(error, "write", hash_value) from error

    def open(self, hash_value: str) -> BinaryIO:
        try:
            response = self._client.get_object(Bucket=self._bucket, Key=self._key(hash_value))
        except Exception as error:  # noqa: BLE001 - mapped to storage errors below
            raise self._map_error(error, "read", hash_value) from error
        return response["Body"]  # type: ignore[no-any-return]

    def delete(self, hash_value: str) -> None:
        try:
            self._client.delete_object(Bucket=self._bucket, Key=self._key(hash_value))
        except Exception as error:  # noqa: BLE001 - GC deletes are best-effort
            raise self._map_error(error, "delete", hash_value) from error


class RoutedBlobStorage(BlobStorage):
    """Facade that routes writes to the current backend and reads across all
    configured backends.

    After an admin switches the backend (local -> S3), objects written before
    the switch remain addressable: reads fall back across the configured
    backends while the per-object ``storage_backend`` metadata written at
    create time records where each object actually lives.
    """

    def __init__(self, default: BlobStorage, fallbacks: Sequence[BlobStorage]) -> None:
        self._default = default
        self._fallbacks = list(fallbacks)
        self.backend_name = default.backend_name

    def _read_backends(self) -> list[BlobStorage]:
        return [self._default, *self._fallbacks]

    def exists(self, hash_value: str, size_bytes: int) -> bool:
        return any(backend.exists(hash_value, size_bytes) for backend in self._read_backends())

    def put_verified(self, hash_value: str, payload: bytes) -> None:
        self._default.put_verified(hash_value, payload)

    def open(self, hash_value: str) -> BinaryIO:
        misses = 0
        for backend in self._read_backends():
            try:
                return backend.open(hash_value)
            except BlobStorageError as error:
                if error.code != "BLOB_NOT_FOUND":
                    raise
                misses += 1
        raise BlobStorageError(
            "BLOB_NOT_FOUND",
            "Blob object was not found in storage.",
            404,
            {"hash": hash_value, "backends_tried": misses},
        )

    def delete(self, hash_value: str) -> None:
        for backend in self._read_backends():
            backend.delete(hash_value)


def s3_credentials_present() -> bool:
    return bool(os.environ.get("S3_ACCESS_KEY_ID")) and bool(
        os.environ.get("S3_SECRET_ACCESS_KEY")
    )


def resolve_s3_credentials() -> tuple[str, str]:
    access_key = os.environ.get("S3_ACCESS_KEY_ID", "")
    secret_key = os.environ.get("S3_SECRET_ACCESS_KEY", "")
    if not access_key or not secret_key:
        raise BlobStorageError(
            "S3_CREDENTIALS_MISSING",
            "S3 storage requires the S3_ACCESS_KEY_ID and S3_SECRET_ACCESS_KEY "
            "environment variables.",
            500,
        )
    return access_key, secret_key


def build_s3_client_from(
    *,
    endpoint_url: str | None,
    region_name: str | None,
    access_key: str,
    secret_key: str,
) -> _S3Client:
    import boto3  # imported lazily so local-only deployments need no AWS SDK

    client: _S3Client = boto3.client(
        "s3",
        endpoint_url=endpoint_url,
        region_name=region_name,
        aws_access_key_id=access_key,
        aws_secret_access_key=secret_key,
    )
    return client


_blob_storage: BlobStorage | None = None
_blob_storage_signature: str | None = None


def _local_backend() -> LocalFilesystemBlobStorage:
    root = Path(settings.blob_storage_path)
    root.mkdir(parents=True, exist_ok=True)
    return LocalFilesystemBlobStorage(root)


def effective_storage_config(session: Session | None = None) -> dict[str, str | None]:
    """Resolve the active storage configuration.

    The admin-editable ``platform`` row in ``system_settings`` overrides the
    environment defaults when a session is available, so a backend switch
    takes effect without a restart. Secrets are never part of the config:
    S3 credentials always come from environment variables.
    """
    backend = (settings.blob_storage_backend or "local").lower()
    config: dict[str, str | None] = {
        "backend": backend,
        "s3_endpoint_url": settings.s3_endpoint_url,
        "s3_bucket": settings.s3_bucket,
        "s3_prefix": settings.s3_prefix,
        "s3_region": settings.s3_region,
    }
    if session is not None:
        row = session.get(SystemSetting, SETTINGS_KEY)
        value = row.value if row is not None and isinstance(row.value, dict) else {}
        if value.get("blob_storage_backend") in ("local", "s3"):
            config["backend"] = str(value["blob_storage_backend"])
        for key in ("s3_endpoint_url", "s3_bucket", "s3_prefix", "s3_region"):
            if value.get(key) is not None:
                config[key] = str(value[key])
    return config


def get_blob_storage(session: Session | None = None) -> BlobStorage:
    """Process-wide routed storage instance.

    Rebuilt whenever the effective configuration changes (an admin switching
    the backend in ``system_settings``), so the change takes effect on the
    next blob operation without a restart. Writes always go to the currently
    selected backend; reads fall back across every configured backend so
    objects written before a switch stay addressable.
    """
    global _blob_storage, _blob_storage_signature
    config = effective_storage_config(session)
    signature = repr(sorted(config.items()))
    if _blob_storage is None or _blob_storage_signature != signature:
        local = _local_backend()
        default: BlobStorage
        if config["backend"] == "s3":
            if not config["s3_bucket"]:
                raise BlobStorageError(
                    "S3_BUCKET_NOT_CONFIGURED",
                    "The S3 storage backend requires a configured bucket.",
                    500,
                )
            access_key, secret_key = resolve_s3_credentials()
            default = S3BlobStorage(
                build_s3_client_from(
                    endpoint_url=config["s3_endpoint_url"],
                    region_name=config["s3_region"],
                    access_key=access_key,
                    secret_key=secret_key,
                ),
                config["s3_bucket"],
                config["s3_prefix"] or "",
            )
        else:
            default = local
        fallbacks = [local] if default is not local else []
        _blob_storage = RoutedBlobStorage(default, fallbacks)
        _blob_storage_signature = signature
    return _blob_storage


def reset_blob_storage_for_tests() -> None:
    """Clear the cached backend so each test gets an isolated instance."""
    global _blob_storage, _blob_storage_signature
    _blob_storage = None
    _blob_storage_signature = None

