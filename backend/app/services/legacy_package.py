"""Legacy Skill package synthesis (plan §17).

Browser-originated Skills carry legacy ``content`` (``instructions`` /
``skill_markdown``) and no immutable package. The desktop sync protocol,
however, transfers complete packages keyed by a manifest hash, and the
change feed ships ``package_manifest_hash`` with every upsert.

This module closes that gap by synthesizing a minimal package — a single
``SKILL.md`` entrypoint rendered from the legacy content — as real,
verified, content-addressed blobs:

- the entrypoint bytes follow the same front-matter shape the template
  service renders (``name``/``description`` YAML + body);
- the manifest uses the exact desktop snapshot schema (``format_version``
  1, strictly sorted ``files`` with canonical sha256 refs), so the desktop
  can read it with its normal manifest parser and materialize the
  workspace;
- every blob (entrypoint + manifest) goes through
  :func:`register_verified_blob`, so a synthesized hash is addressable by
  the existing download endpoint.

Synthesis is deterministic and idempotent: identical content always
produces identical hashes, and re-running is a no-op against the registry.

The intended call sites are the shared domain mutation path (so every new
browser write carries a package before its change event fires) and a
one-time backfill for pre-existing rows.
"""

from __future__ import annotations

import hashlib
import json
from collections.abc import Callable
from typing import Any

from sqlalchemy.orm import Session

from app.services.blob_registry import register_verified_blob
from app.services.blob_storage import BlobStorage
from app.services.package_manifest import SKILL_ENTRYPOINT, SNAPSHOT_FORMAT_VERSION


def _entrypoint_markdown(slug: str, description: str, body: str) -> str:
    safe_description = " ".join(description.splitlines()).strip()
    yaml_description = json.dumps(safe_description, ensure_ascii=False)
    return f"---\nname: {slug}\ndescription: {yaml_description}\n---\n\n{body.strip()}\n"


def _legacy_body(content: dict[str, Any]) -> str:
    """Pick the best legacy body: skill_markdown first, then instructions.

    A ``skill_markdown`` body is already a full document (often with
    front-matter); ``instructions`` is the plain workflow body the web
    editor maintains. Both are acceptable entrypoint bodies.
    """
    markdown = content.get("skill_markdown")
    if isinstance(markdown, str) and markdown.strip():
        return markdown
    instructions = content.get("instructions")
    if isinstance(instructions, str):
        return instructions
    return ""


def synthesize_legacy_package(
    session: Session,
    storage: BlobStorage,
    *,
    slug: str,
    name: str,
    description: str,
    content: dict[str, Any],
) -> str | None:
    """Build and register the minimal package for a legacy content map.

    Returns the manifest hash, or ``None`` when the content carries no
    usable body (the Skill then stays package-less and the desktop treats
    it as metadata-only, exactly as before).

    The returned hash is safe to store on the Skill/version rows: both
    blobs are durably stored and verified before this function returns,
    and the registry rows are part of the caller's transaction.
    """
    del name  # the entrypoint's YAML carries the slug; name stays in metadata
    body = _legacy_body(content)
    if not body.strip():
        return None

    entrypoint = _entrypoint_markdown(slug, description, body)
    entrypoint_bytes = entrypoint.encode("utf-8")
    entrypoint_hash = "sha256:" + hashlib.sha256(entrypoint_bytes).hexdigest()
    register_verified_blob(session, storage, entrypoint_hash, entrypoint_bytes)

    manifest = {
        "format_version": SNAPSHOT_FORMAT_VERSION,
        "files": [
            {
                "path": SKILL_ENTRYPOINT,
                "blobHash": entrypoint_hash,
                "sizeBytes": len(entrypoint_bytes),
            }
        ],
    }
    manifest_bytes = _canonical_manifest_bytes(manifest)
    manifest_hash = "sha256:" + hashlib.sha256(manifest_bytes).hexdigest()
    register_verified_blob(session, storage, manifest_hash, manifest_bytes)
    return manifest_hash


def _canonical_manifest_bytes(manifest: dict[str, Any]) -> bytes:
    """Serialize the manifest the way the desktop hashes its snapshots.

    Python's ``json.dumps`` with ``separators=(",", ":")`` and no ``sort_keys``
    emits the same byte layout as the desktop's serde_json compact writer for
    this fixed-shape manifest, so the desktop can recompute the hash from the
    downloaded bytes.
    """
    return json.dumps(manifest, separators=(",", ":")).encode("utf-8")


def manifest_matches_entrypoint(
    slug: str,
    description: str,
    content: dict[str, Any],
) -> Callable[[str, bytes], bool]:
    """Return a checker verifying a (hash, bytes) pair equals the synthesis output.

    Test seam: lets tests recompute the expected entrypoint/manifest pair
    without re-registering blobs.
    """
    body = _legacy_body(content)
    entrypoint = _entrypoint_markdown(slug, description, body)
    entrypoint_bytes = entrypoint.encode("utf-8")
    entrypoint_hash = "sha256:" + hashlib.sha256(entrypoint_bytes).hexdigest()
    manifest = {
        "format_version": SNAPSHOT_FORMAT_VERSION,
        "files": [
            {
                "path": SKILL_ENTRYPOINT,
                "blobHash": entrypoint_hash,
                "sizeBytes": len(entrypoint_bytes),
            }
        ],
    }
    manifest_bytes = _canonical_manifest_bytes(manifest)
    manifest_hash = "sha256:" + hashlib.sha256(manifest_bytes).hexdigest()
    expected = {manifest_hash: manifest_bytes, entrypoint_hash: entrypoint_bytes}

    def check(hash_value: str, payload: bytes) -> bool:
        return expected.get(hash_value) == payload

    return check
