import os

os.environ["MODEL_IMPL"] = "rule"  # 계약 검증을 위해 rule 구현체로 고정

import pytest
from fastapi.testclient import TestClient

from api.app import app

client = TestClient(app)


def test_health():
    """/health 는 200 과 status=ok, 현재 구현체 이름을 돌려준다."""
    res = client.get("/health")
    assert res.status_code == 200
    assert res.json() == {"status": "ok", "model": "rule"}


def test_eta_response_shape():
    """/eta 정상 응답은 minutes(숫자)와 model(문자열) 필드만 갖는다."""
    res = client.post("/eta", json={"distance_km": 1, "pending_orders": 1})
    assert res.status_code == 200
    body = res.json()
    assert set(body) == {"minutes", "model"}
    assert isinstance(body["minutes"], (int, float))
    assert body["model"] == "rule"


@pytest.mark.parametrize(
    ("distance_km", "pending_orders", "expected"),
    [
        (0, 0, 10.0),          # 경계값: 모두 0
        (2.5, 3, 23.5),        # 일반 입력
        (1000, 10000, 23010.0),  # 큰 값
    ],
)
def test_eta_known_values(distance_km, pending_orders, expected):
    """알려진 입력에 대해 /eta 가 예상 분(minutes)을 돌려준다."""
    res = client.post("/eta", json={"distance_km": distance_km, "pending_orders": pending_orders})
    assert res.status_code == 200
    assert res.json()["minutes"] == pytest.approx(expected)


@pytest.mark.parametrize(
    "payload",
    [
        {"distance_km": -1, "pending_orders": 0},       # 음수 거리
        {"distance_km": 1, "pending_orders": -1},       # 음수 주문 수
        {"distance_km": "abc", "pending_orders": 0},    # 숫자가 아닌 거리
        {"distance_km": 1, "pending_orders": "many"},   # 숫자가 아닌 주문 수
        {"distance_km": 1, "pending_orders": 1.5},      # 정수가 아닌 주문 수
        {"distance_km": 1},                             # 필드 누락
        {},                                             # 빈 본문
    ],
)
def test_eta_invalid_input_returns_422(payload):
    """잘못된 입력(음수, 숫자 아님, 누락)은 422 를 돌려준다."""
    res = client.post("/eta", json=payload)
    assert res.status_code == 422
