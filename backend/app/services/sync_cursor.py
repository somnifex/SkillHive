import base64

_CURSOR_PREFIX = "v1."
_CURSOR_BYTES = 8
_MAX_CURSOR_LENGTH = 64


class SyncCursorError(ValueError):
    pass


def encode_sync_cursor(sequence: int) -> str:
    if sequence < 0 or sequence > (2**64 - 1):
        raise SyncCursorError("sync cursor sequence is out of range")
    payload = sequence.to_bytes(_CURSOR_BYTES, byteorder="big", signed=False)
    encoded = base64.urlsafe_b64encode(payload).decode("ascii").rstrip("=")
    return f"{_CURSOR_PREFIX}{encoded}"


def decode_sync_cursor(cursor: str | None) -> int:
    if cursor is None or cursor == "":
        return 0
    if len(cursor) > _MAX_CURSOR_LENGTH or not cursor.startswith(_CURSOR_PREFIX):
        raise SyncCursorError("unsupported or malformed sync cursor")

    encoded = cursor.removeprefix(_CURSOR_PREFIX)
    if not encoded:
        raise SyncCursorError("malformed sync cursor")
    padding = "=" * (-len(encoded) % 4)
    try:
        payload = base64.b64decode(
            encoded + padding,
            altchars=b"-_",
            validate=True,
        )
    except (ValueError, base64.binascii.Error) as error:
        raise SyncCursorError("malformed sync cursor") from error
    if len(payload) != _CURSOR_BYTES:
        raise SyncCursorError("malformed sync cursor")
    return int.from_bytes(payload, byteorder="big", signed=False)
