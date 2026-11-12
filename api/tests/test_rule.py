import pytest

from api.predictors.rule import estimate_delivery_minutes


@pytest.mark.parametrize(
    ("distance_km", "pending_orders", "expected"),
    [
        (0, 0, 10.0),
        (1, 0, 13.0),
        (0, 1, 12.0),
        (2.5, 3, 23.5),
        (1000, 10000, 23010.0),
    ],
)
def test_known_values(distance_km, pending_orders, expected):
    """알려진 입력에 대해 10 + 3*거리 + 2*대기주문 값을 돌려준다."""
    assert estimate_delivery_minutes(distance_km, pending_orders) == pytest.approx(expected)


def test_returns_float():
    """반환값은 float 이다."""
    assert isinstance(estimate_delivery_minutes(1, 1), float)
