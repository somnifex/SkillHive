from typing import cast

from app.core.security import hash_password
from app.db.seed import seed_database
from app.models import User
from fastapi.testclient import TestClient
from sqlalchemy.orm import Session


def register_user(client: TestClient, username: str = "alice") -> dict[str, object]:
    response = client.post(
        "/api/v1/auth/register",
        json={
            "username": username,
            "display_name": "Alice",
            "email": f"{username}@example.com",
            "password": "Strong123!",
        },
    )
    assert response.status_code == 201
    return cast(dict[str, object], response.json())


def test_register_login_refresh_logout_flow(client: TestClient) -> None:
    user = register_user(client)
    assert user["username"] == "alice"
    assert "password_hash" not in user

    login = client.post(
        "/api/v1/auth/login",
        json={"username": "alice", "password": "Strong123!"},
    )
    assert login.status_code == 200
    token = login.json()["access_token"]
    assert "skillhive_refresh" in login.cookies

    me = client.get("/api/v1/auth/me", headers={"Authorization": f"Bearer {token}"})
    assert me.status_code == 200
    assert me.json()["email"] == "alice@example.com"

    refreshed = client.post("/api/v1/auth/refresh")
    assert refreshed.status_code == 200
    assert refreshed.json()["access_token"] != token

    old_refresh = login.cookies["skillhive_refresh"]
    replay = client.post(
        "/api/v1/auth/refresh",
        cookies={"skillhive_refresh": old_refresh},
    )
    assert replay.status_code == 401
    assert replay.json()["error"]["code"] == "SESSION_REVOKED"

    logout = client.post("/api/v1/auth/logout")
    assert logout.status_code == 200
    assert client.post("/api/v1/auth/refresh").status_code == 401


def test_disabled_user_cannot_login(client: TestClient, db_session: Session) -> None:
    db_session.add(
        User(
            username="disabled",
            display_name="Disabled",
            email="disabled@example.com",
            password_hash=hash_password("Strong123!"),
            status="disabled",
        )
    )
    db_session.commit()
    response = client.post(
        "/api/v1/auth/login",
        json={"username": "disabled", "password": "Strong123!"},
    )
    assert response.status_code == 403
    assert response.json()["error"]["code"] == "USER_DISABLED"


def test_protected_route_rejects_anonymous_user(client: TestClient) -> None:
    response = client.get("/api/v1/auth/me")
    assert response.status_code == 401
    assert response.json()["error"]["code"] == "NOT_AUTHENTICATED"


def test_login_failure_lockout(client: TestClient) -> None:
    register_user(client, "limited")
    for _ in range(5):
        response = client.post(
            "/api/v1/auth/login",
            json={"username": "limited", "password": "wrong"},
        )
        assert response.status_code == 401
    locked = client.post(
        "/api/v1/auth/login",
        json={"username": "limited", "password": "Strong123!"},
    )
    assert locked.status_code == 429
    assert locked.json()["error"]["code"] == "LOGIN_RATE_LIMITED"


def test_seeded_default_accounts_can_login(
    client: TestClient,
    db_session: Session,
) -> None:
    seed_database(db_session)
    for username, password in (("admin", "Admin123!"), ("howie", "User123!")):
        response = client.post(
            "/api/v1/auth/login",
            json={"username": username, "password": password},
        )
        assert response.status_code == 200
        assert response.json()["user"]["username"] == username
