"""Server-side snapshot manifest validation for M2.2.

Mirrors the desktop snapshot validation constraints (frontend/src-tauri
``skill_snapshot.rs``) so a manifest accepted by the desktop cannot be
rejected by the server, and an invalid manifest cannot commit a Skill
mutation:

- format version 1;
- ``SKILL.md`` entrypoint required;
- strict, sorted, unique portable paths;
- canonical lowercase sha256 blob identifiers;
- bounded file count, file size and package size;
- Windows-portable filename rules (reserved names, trailing dots/spaces,
  forbidden separators/control characters).
"""

from __future__ import annotations

import json
import re
from collections.abc import Callable
from typing import Any, cast

from app.core.exceptions import AppError
from app.schemas.sync import MAX_BLOB_BYTES, MAX_PACKAGE_BYTES

SNAPSHOT_FORMAT_VERSION = 1
SKILL_ENTRYPOINT = "SKILL.md"
MAX_SNAPSHOT_FILES = 10_000
MAX_SEGMENT_BYTES = 255
MAX_MANIFEST_BYTES = 16 * 1024 * 1024

_SHA256_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
_RESERVED_STEMS = (
    {"CON", "PRN", "AUX", "NUL"}
    | {f"COM{i}" for i in range(1, 10)}
    | {f"LPT{i}" for i in range(1, 10)}
)


def _manifest_error(code: str, message: str, status_code: int = 400, **details: Any) -> AppError:
    return AppError(code, message, status_code, details or None)


def validate_snapshot_manifest_bytes(manifest_bytes: bytes) -> list[dict[str, Any]]:
    """Parse and fully validate a snapshot manifest payload.

    Returns the normalized file entry list. The manifest bytes are exactly
    what the caller hashed into ``package_manifest_hash``; the caller must
    separately verify that every referenced blob exists (see
    :func:`validate_package_closure`).
    """
    if len(manifest_bytes) > MAX_MANIFEST_BYTES:
        raise AppError("MANIFEST_TOO_LARGE", "Snapshot manifest exceeds size limits.", 413)

    try:
        manifest = json.loads(manifest_bytes)
    except (ValueError, UnicodeDecodeError) as exc:
        raise AppError(
            "MANIFEST_INVALID_JSON", "Snapshot manifest is not valid JSON.", 400
        ) from exc

    if not isinstance(manifest, dict):
        raise AppError("MANIFEST_INVALID", "Snapshot manifest must be a JSON object.", 400)
    if manifest.get("format_version") != SNAPSHOT_FORMAT_VERSION:
        raise AppError(
            "MANIFEST_UNSUPPORTED_VERSION",
            "Unsupported snapshot manifest format version.",
            400,
            {"format_version": manifest.get("format_version")},
        )
    files = manifest.get("files")
    if not isinstance(files, list):
        raise AppError("MANIFEST_INVALID", "Snapshot manifest files must be a list.", 400)
    if len(files) > MAX_SNAPSHOT_FILES:
        raise AppError(
            "MANIFEST_TOO_MANY_FILES",
            f"Snapshot declares more than {MAX_SNAPSHOT_FILES} files.",
            400,
        )

    previous: str | None = None
    has_entrypoint = False
    total_bytes = 0
    normalized: list[dict[str, Any]] = []
    for entry in files:
        if not isinstance(entry, dict):
            raise AppError("MANIFEST_INVALID_ENTRY", "Snapshot file entry must be an object.", 400)
        path = entry.get("path")
        blob_hash = entry.get("blobHash", entry.get("blob_hash"))
        size = entry.get("sizeBytes", entry.get("size_bytes"))

        if not isinstance(path, str) or not isinstance(size, int) or isinstance(size, bool):
            raise AppError("MANIFEST_INVALID_ENTRY", "Snapshot file entry is malformed.", 400)
        if not isinstance(blob_hash, str) or not _SHA256_RE.match(blob_hash):
            raise AppError(
                "MANIFEST_INVALID_HASH",
                "Snapshot blob hash is not a canonical sha256 identifier.",
                400,
            )
        if size < 0 or size > MAX_BLOB_BYTES:
            raise AppError(
                "MANIFEST_FILE_TOO_LARGE", "Snapshot file exceeds the per-file limit.", 413
            )
        validate_portable_path(path)

        if previous is not None and previous >= path:
            raise AppError(
                "MANIFEST_NOT_SORTED",
                "Snapshot manifest paths must be strictly sorted and unique.",
                400,
            )
        previous = path
        if path == SKILL_ENTRYPOINT:
            has_entrypoint = True
        total_bytes += size
        if total_bytes > MAX_PACKAGE_BYTES:
            raise AppError(
                "MANIFEST_PACKAGE_TOO_LARGE", "Snapshot package exceeds the total size limit.", 413
            )
        normalized.append({"path": path, "blob_hash": blob_hash, "size_bytes": size})

    if not has_entrypoint:
        raise AppError(
            "MANIFEST_MISSING_ENTRYPOINT",
            f"Snapshot package must contain {SKILL_ENTRYPOINT}.",
            400,
        )
    return normalized


def validate_package_closure(
    manifest_files: list[dict[str, Any]],
    blob_exists: Callable[[str, int], bool],
) -> tuple[int, int]:
    """Verify every referenced blob exists and its size matches.

    Returns ``(file_count, total_bytes)`` on success. Raises ``BLOB_MISSING``
    naming the first absent or size-divergent blob otherwise.
    """
    total_bytes = 0
    for entry in manifest_files:
        hash_value = cast("str", entry["blob_hash"])
        size = cast("int", entry["size_bytes"])
        if not blob_exists(hash_value, size):
            raise AppError(
                "BLOB_MISSING",
                "Package references a blob the server does not hold.",
                409,
                {"hash": hash_value},
            )
        total_bytes += size
    return len(manifest_files), total_bytes


def validate_portable_path(path: str) -> None:
    """Enforce the same portable path rules as the desktop snapshot capture."""
    if not path or path.startswith("/") or "\\" in path:
        raise AppError("MANIFEST_INVALID_PATH", f"Invalid snapshot path: {path!r}", 400)
    for segment in path.split("/"):
        _validate_portable_segment(segment, path)


def _validate_portable_segment(segment: str, full_path: str) -> None:
    if (
        not segment
        or segment in {".", ".."}
        or len(segment.encode("utf-8")) > MAX_SEGMENT_BYTES
        or segment.endswith(" ")
        or segment.endswith(".")
    ):
        raise AppError("MANIFEST_INVALID_PATH", f"Invalid snapshot path: {full_path!r}", 400)
    if any(character in segment for character in '<>:"\\|?*') or any(
        ord(character) < 0x20 for character in segment
    ):
        raise AppError("MANIFEST_INVALID_PATH", f"Invalid snapshot path: {full_path!r}", 400)
    stem = segment.split(".", 1)[0].upper()
    if stem in _RESERVED_STEMS:
        raise AppError(
            "MANIFEST_RESERVED_NAME",
            f"Snapshot path uses a Windows-reserved name: {segment!r}",
            400,
        )
