from app.main import app
from fastapi.testclient import TestClient


def test_health_check() -> None:
    response = TestClient(app).get("/api/v1/health")
    assert response.status_code == 200
    assert response.json() == {"status": "ok", "service": "skillhive-api"}
