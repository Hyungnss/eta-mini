"""예측 일치(parity) 검사: Python API 와 다른 언어 서비스(Go, Rust)의 예측이 같은지 비교한다.

서버가 떠 있어야 하므로 pytest 테스트가 아니라 스크립트로 실행한다.
    uv run python -m training.check_parity

환경 변수
  PY_URL   기준 Python API (학습한 원본 모델, MODEL_IMPL=sklearn). 기본 http://127.0.0.1:8080
  TARGETS  비교 대상 "이름=URL" 을 쉼표로 구분. 기본 go=http://127.0.0.1:8081,rust=http://127.0.0.1:8082

판정: 대상별 최대 오차 < TOLERANCE 분이면 PASS. 하나라도 FAIL 이면 exit 1 (CI 가 멈춘다).
표준 라이브러리만 쓴다.
"""
import json
import os
import random
import sys
import urllib.error
import urllib.request

TOLERANCE = 1e-4  # 분
TIMEOUT = 5       # 초

PY_URL = os.getenv("PY_URL", "http://127.0.0.1:8080")
TARGETS = os.getenv("TARGETS", "go=http://127.0.0.1:8081,rust=http://127.0.0.1:8082")


def parse_targets(spec: str) -> dict[str, str]:
    """"이름=URL,이름=URL" 형식의 문자열을 {이름: URL} 로 바꾼다.

    Args:
        spec: TARGETS 환경 변수 값.

    Returns:
        이름 → URL 딕셔너리.
    """
    targets = {}
    for item in spec.split(","):
        item = item.strip()
        if not item:
            continue
        name, sep, url = item.partition("=")
        if not sep or not name.strip() or not url.strip():
            sys.exit(f"TARGETS 형식 오류: {item!r} (이름=URL 이어야 한다)")
        targets[name.strip()] = url.strip().rstrip("/")
    return targets


def make_inputs() -> list[tuple[float, int]]:
    """비교할 입력을 만든다: 고정 3건 + seed 0 무작위 20건.

    Returns:
        (배달 거리 km, 대기 주문 수) 목록.
    """
    rng = random.Random(0)
    fixed = [(3.0, 2), (0.0, 0), (8.0, 10)]
    rand = [(rng.uniform(0.5, 8.0), rng.randint(0, 10)) for _ in range(20)]
    return fixed + rand


def get_json(url: str) -> dict:
    """GET 요청을 보내고 JSON 응답을 돌려준다."""
    with urllib.request.urlopen(url, timeout=TIMEOUT) as resp:
        return json.load(resp)


def predict(base_url: str, distance_km: float, pending_orders: int) -> float:
    """POST /eta 로 예측을 요청한다.

    Args:
        base_url: 서비스 주소.
        distance_km: 배달 거리(km).
        pending_orders: 대기 주문 수.

    Returns:
        응답의 minutes 값.
    """
    body = json.dumps({"distance_km": distance_km, "pending_orders": pending_orders}).encode()
    req = urllib.request.Request(
        f"{base_url}/eta", data=body, headers={"Content-Type": "application/json"}, method="POST"
    )
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
        return float(json.load(resp)["minutes"])


def main() -> int:
    """기준 예측을 구하고 대상별로 비교해 결과를 출력한다.

    Returns:
        모두 PASS 면 0, 하나라도 FAIL 이면 1.
    """
    base = PY_URL.rstrip("/")
    targets = parse_targets(TARGETS)
    inputs = make_inputs()

    # 기준이 원본 모델이 아니면 비교 자체가 무의미하다
    try:
        model = get_json(f"{base}/health").get("model")
        expected = [predict(base, d, p) for d, p in inputs]
    except (urllib.error.URLError, OSError, ValueError, KeyError) as e:
        print(f"FAIL [python] 기준 서버 {base} 에서 예측을 받지 못함: {e}")
        return 1
    if model != "sklearn":
        print(f"FAIL [python] 기준 서버 모델이 sklearn 이 아니다 (model={model!r}). MODEL_IMPL=sklearn 으로 띄울 것")
        return 1
    print(f"기준: python ({base}, model={model}), 입력 {len(inputs)}건, 허용 오차 {TOLERANCE}분")

    all_ok = True
    for name, url in targets.items():
        try:
            actual = [predict(url, d, p) for d, p in inputs]
        except (urllib.error.URLError, OSError, ValueError, KeyError) as e:
            print(f"FAIL [{name}] {url} 에서 예측을 받지 못함: {e}")
            all_ok = False
            continue

        diffs = [abs(a - e) for a, e in zip(actual, expected)]
        worst = max(range(len(diffs)), key=diffs.__getitem__)
        ok = diffs[worst] < TOLERANCE
        all_ok &= ok
        d, p = inputs[worst]
        print(
            f"{'PASS' if ok else 'FAIL'} [{name}] 최대 오차 {diffs[worst]:.2e}분 "
            f"(입력 거리 {d:.3f}km, 주문 {p}건: python {expected[worst]:.6f} / {name} {actual[worst]:.6f})"
        )

    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
