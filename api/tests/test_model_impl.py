import importlib
import sys
import types

import pytest
from fastapi.testclient import TestClient


@pytest.fixture
def reload_app(monkeypatch):
    """MODEL_IMPL 을 바꿔 api.app 을 다시 불러오고, 끝나면 rule 로 되돌린다."""
    import api.app

    def _reload(impl: str):
        monkeypatch.setenv("MODEL_IMPL", impl)
        return importlib.reload(api.app)

    yield _reload
    monkeypatch.setenv("MODEL_IMPL", "rule")
    importlib.reload(api.app)


def test_health_and_eta_report_selected_impl(reload_app, monkeypatch):
    """MODEL_IMPL 로 고른 구현체 이름이 /health 와 /eta 의 model 에 그대로 나온다."""
    fake = types.ModuleType("api.predictors.fake")
    fake.estimate_delivery_minutes = lambda distance_km, pending_orders: 42.0
    monkeypatch.setitem(sys.modules, "api.predictors.fake", fake)

    client = TestClient(reload_app("fake").app)

    assert client.get("/health").json() == {"status": "ok", "model": "fake"}
    res = client.post("/eta", json={"distance_km": 1, "pending_orders": 1})
    assert res.status_code == 200
    assert res.json() == {"minutes": 42.0, "model": "fake"}


def test_unknown_impl_fails_at_startup(reload_app):
    """존재하지 않는 구현체 이름이면 앱을 불러오는 시점에 실패한다."""
    with pytest.raises(ModuleNotFoundError):
        reload_app("does_not_exist")
