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
import tempfile
from abc import ABC, abstractmethod
from pathlib import Path
from typing import BinaryIO

from app.core.config import settings
from app.core.exceptions import AppError

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


_blob_storage: BlobStorage | None = None


def get_blob_storage() -> BlobStorage:
    """Process-wide storage instance rooted at the configured data directory."""
    global _blob_storage
    if _blob_storage is None:
        root = Path(settings.blob_storage_path)
        root.mkdir(parents=True, exist_ok=True)
        _blob_storage = LocalFilesystemBlobStorage(root)
    return _blob_storage


def reset_blob_storage_for_tests() -> None:
    """Clear the cached backend so each test gets an isolated instance."""
    global _blob_storage
    _blob_storage = None
