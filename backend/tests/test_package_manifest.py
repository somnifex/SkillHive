"""Server-side snapshot manifest validation tests for M2.2.

Validates the same constraints as the desktop snapshot policy: entrypoint,
sorted unique portable paths, canonical hashes, size bounds, and the
Windows-portable filename rules. Also covers package closure verification.
"""

import json
from typing import Any

import pytest
from app.core.exceptions import AppError
from app.services.package_manifest import (
    validate_package_closure,
    validate_portable_path,
    validate_snapshot_manifest_bytes,
)

_ENTRYPOINT = "SKILL.md"
_HASH = f"sha256:{'a' * 64}"
_HASH_B = f"sha256:{'b' * 64}"


def _manifest(*entries: dict[str, Any]) -> bytes:
    return json.dumps({"format_version": 1, "files": list(entries)}).encode("utf-8")


def _entry(path: str = _ENTRYPOINT, *, size: int = 8, blob_hash: str = _HASH) -> dict[str, Any]:
    return {"path": path, "blobHash": blob_hash, "sizeBytes": size}


def test_accepts_minimal_valid_manifest() -> None:
    files = validate_snapshot_manifest_bytes(_manifest(_entry()))
    assert files == [{"path": _ENTRYPOINT, "blob_hash": _HASH, "size_bytes": 8}]


def test_rejects_invalid_json() -> None:
    with pytest.raises(AppError) as error:
        validate_snapshot_manifest_bytes(b"{not json")
    assert error.value.code == "MANIFEST_INVALID_JSON"


def test_rejects_wrong_format_version() -> None:
    payload = json.dumps({"format_version": 2, "files": []}).encode("utf-8")
    with pytest.raises(AppError) as raised:
        validate_snapshot_manifest_bytes(payload)
    assert raised.value.code == "MANIFEST_UNSUPPORTED_VERSION"


def test_rejects_missing_entrypoint() -> None:
    with pytest.raises(AppError) as raised:
        validate_snapshot_manifest_bytes(_manifest(_entry(path="scripts/run.py")))
    assert raised.value.code == "MANIFEST_MISSING_ENTRYPOINT"


def test_rejects_unsorted_paths() -> None:
    with pytest.raises(AppError) as raised:
        validate_snapshot_manifest_bytes(
            _manifest(_entry(path="scripts/b.py"), _entry(path="scripts/a.py"))
        )
    assert raised.value.code == "MANIFEST_NOT_SORTED"


def test_rejects_duplicate_paths() -> None:
    with pytest.raises(AppError) as raised:
        validate_snapshot_manifest_bytes(_manifest(_entry(), _entry()))
    assert raised.value.code == "MANIFEST_NOT_SORTED"


def test_rejects_oversized_file() -> None:
    with pytest.raises(AppError) as raised:
        validate_snapshot_manifest_bytes(_manifest(_entry(size=65 * 1024 * 1024)))
    assert raised.value.code == "MANIFEST_FILE_TOO_LARGE"


@pytest.mark.parametrize(
    "path",
    [
        "../escape/SKILL.md",
        "/absolute/SKILL.md",
        "back\\slash/SKILL.md",
        "trailing./SKILL.md",
        "trailing /SKILL.md",
        "CON/SKILL.md",
        "script?.py",
        "a:b.md",
        "pipe|.md",
        "",
    ],
)
def test_rejects_invalid_portable_paths(path: str) -> None:
    with pytest.raises(AppError):
        validate_portable_path(path)


@pytest.mark.parametrize(
    "path",
    ["SKILL.md", "scripts/run.py", "references/info.md", "assets/fixture.bin"],
)
def test_accepts_valid_portable_paths(path: str) -> None:
    validate_portable_path(path)


def test_package_closure_verifies_every_blob() -> None:
    files = validate_snapshot_manifest_bytes(
        _manifest(_entry(), _entry(path="scripts/x.py", blob_hash=_HASH_B))
    )

    assert validate_package_closure(files, lambda _hash, _size: True) == (2, 16)


def test_package_closure_reports_missing_blob() -> None:
    files = validate_snapshot_manifest_bytes(_manifest(_entry()))

    with pytest.raises(AppError) as raised:
        validate_package_closure(files, lambda _hash, _size: False)
    assert raised.value.code == "BLOB_MISSING"
    assert raised.value.details == {"hash": _HASH}
