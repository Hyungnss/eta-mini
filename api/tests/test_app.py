import math
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


# ---------------------------------------------------------------------------
# 엣지 케이스
# ---------------------------------------------------------------------------


# 서버 예외를 500 응답으로 받기 위한 클라이언트 (예외가 테스트로 새어 나오지 않게)
safe_client = TestClient(app, raise_server_exceptions=False)


def _post_raw(body: str):
    """JSON 직렬화를 거치지 않고 본문 문자열을 그대로 보낸다 (NaN/Infinity 등)."""
    return safe_client.post("/eta", content=body, headers={"content-type": "application/json"})


@pytest.mark.parametrize(
    ("distance_km", "pending_orders", "expected"),
    [
        (5, 0, 25.0),   # 경계값: 주문 수만 0
        (0, 5, 20.0),   # 경계값: 거리만 0
        (-0.0, 0, 10.0),  # 경계값: 음의 0 은 0 으로 취급
    ],
)
def test_eta_zero_boundaries(distance_km, pending_orders, expected):
    """한쪽만 0 이거나 -0.0 인 입력도 200 과 예상 분을 돌려준다."""
    res = client.post("/eta", json={"distance_km": distance_km, "pending_orders": pending_orders})
    assert res.status_code == 200
    assert res.json()["minutes"] == pytest.approx(expected)


@pytest.mark.parametrize(
    "payload",
    [
        {"distance_km": -0.0001, "pending_orders": 0},   # 0 바로 아래 음수
        {"distance_km": None, "pending_orders": 0},      # null 거리
        {"distance_km": 1, "pending_orders": None},      # null 주문 수
        [1, 2],                                          # 객체가 아닌 본문
    ],
)
def test_eta_more_invalid_input_returns_422(payload):
    """0 바로 아래 음수, null, 객체가 아닌 본문은 422 를 돌려준다."""
    res = client.post("/eta", json=payload)
    assert res.status_code == 422


def test_eta_malformed_json_returns_422():
    """깨진 JSON 본문은 422 를 돌려준다."""
    res = _post_raw('{"distance_km": 1,')
    assert res.status_code == 422


def test_eta_422_points_to_invalid_field():
    """422 응답의 detail 에 어느 필드가 잘못됐는지(loc) 들어 있다."""
    res = client.post("/eta", json={"distance_km": -1, "pending_orders": 0})
    assert res.status_code == 422
    locs = [err["loc"] for err in res.json()["detail"]]
    assert ["body", "distance_km"] in locs


@pytest.mark.parametrize(
    ("method", "path"),
    [("get", "/eta"), ("post", "/health")],
)
def test_wrong_http_method_returns_405(method, path):
    """허용되지 않은 HTTP 메서드는 405 를 돌려준다."""
    res = getattr(client, method)(path)
    assert res.status_code == 405


# --- 현재 동작을 계약으로 고정 (정책이 바뀌면 여기를 함께 고친다) ---

def test_eta_accepts_numeric_strings():
    """숫자 문자열("1.5", "2")은 숫자로 변환되어 계산된다."""
    res = client.post("/eta", json={"distance_km": "1.5", "pending_orders": "2"})
    assert res.status_code == 200
    assert res.json()["minutes"] == pytest.approx(18.5)


def test_eta_accepts_integral_float_for_orders():
    """pending_orders 에 소수부가 0 인 실수(2.0)는 정수로 받아들인다."""
    res = client.post("/eta", json={"distance_km": 1, "pending_orders": 2.0})
    assert res.status_code == 200
    assert res.json()["minutes"] == pytest.approx(17.0)


def test_eta_ignores_unknown_fields():
    """정의되지 않은 필드는 무시하고 정상 응답한다."""
    res = client.post("/eta", json={"distance_km": 1, "pending_orders": 0, "extra": 1})
    assert res.status_code == 200
    assert set(res.json()) == {"minutes", "model"}
    assert res.json()["minutes"] == pytest.approx(13.0)


def test_eta_huge_pending_orders():
    """아주 큰 정수 주문 수(10^26)도 유한한 숫자로 계산된다."""
    res = _post_raw('{"distance_km": 0, "pending_orders": 100000000000000000000000000}')
    assert res.status_code == 200
    assert res.json()["minutes"] == pytest.approx(2e26)


# --- 알려진 버그: 수정되면 strict xfail 이 XPASS 로 실패해 표시를 떼라고 알려 준다 ---

@pytest.mark.xfail(strict=True, reason="bool 이 1/0 으로 변환되어 200 이 된다")
@pytest.mark.parametrize(
    "payload",
    [
        {"distance_km": True, "pending_orders": 0},
        {"distance_km": 1, "pending_orders": False},
    ],
)
def test_eta_bool_returns_422(payload):
    """불리언 값은 숫자로 취급하지 않고 422 를 돌려준다."""
    res = client.post("/eta", json=payload)
    assert res.status_code == 422


@pytest.mark.xfail(strict=True, reason="NaN 이 들어간 검증 오류를 직렬화하지 못해 500 이 난다")
def test_eta_nan_returns_422():
    """distance_km 에 NaN 이 오면 422 를 돌려준다."""
    res = _post_raw('{"distance_km": NaN, "pending_orders": 0}')
    assert res.status_code == 422


@pytest.mark.xfail(strict=True, reason="ge=0 이 Infinity 를 막지 않아 minutes=null 로 200 이 된다")
def test_eta_infinity_returns_422():
    """distance_km 에 Infinity 가 오면 422 를 돌려준다."""
    res = _post_raw('{"distance_km": Infinity, "pending_orders": 0}')
    assert res.status_code == 422


@pytest.mark.xfail(strict=True, reason="3*1e308 이 inf 로 넘쳐 minutes=null 로 200 이 된다")
def test_eta_overflow_never_returns_non_finite_minutes():
    """계산이 넘칠 만큼 큰 거리라도 null/inf 를 돌려주지 않는다 (422 또는 유한한 숫자)."""
    res = _post_raw('{"distance_km": 1e308, "pending_orders": 0}')
    assert res.status_code in (200, 422)
    if res.status_code == 200:
        minutes = res.json()["minutes"]
        assert isinstance(minutes, (int, float)) and math.isfinite(minutes)
