package main

import (
	"encoding/json"
	"math"
	"net/http/httptest"
	"strings"
	"testing"
)

// rulePredict 는 api/predictors/rule.py 와 같은 식이다. 계약(검증·응답 형식) 확인용으로 모델 대신 쓴다.
func rulePredict(d, p float64) (float64, error) { return 10 + 3*d + 2*p, nil }

// testIndex 는 루트 페이지 템플릿의 자리표시자만 남긴 축약본이다.
const testIndex = `<p>모델: {{model}}</p><nav>{{docs_link}}</nav>`

func do(t *testing.T, method, path, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(method, path, strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	rec := httptest.NewRecorder()
	newMux("rule", testIndex, rulePredict).ServeHTTP(rec, req)
	return rec
}

// TestHealth: /health 는 200 과 status=ok, model 이름을 돌려준다.
func TestHealth(t *testing.T) {
	rec := do(t, "GET", "/health", "")
	if rec.Code != 200 || strings.TrimSpace(rec.Body.String()) != `{"model":"rule","status":"ok"}` {
		t.Fatalf("got %d %s", rec.Code, rec.Body)
	}
}

// TestEtaKnownValues: 정상·경계·관대한 입력에 대해 200 과 예상 분을 돌려준다.
func TestEtaKnownValues(t *testing.T) {
	cases := map[string]float64{
		`{"distance_km": 0, "pending_orders": 0}`:                           10,
		`{"distance_km": 2.5, "pending_orders": 3}`:                         23.5,
		`{"distance_km": 1000, "pending_orders": 10000}`:                    23010,
		`{"distance_km": -0.0, "pending_orders": 0}`:                        10,
		`{"distance_km": "1.5", "pending_orders": "2"}`:                     18.5,
		`{"distance_km": 1, "pending_orders": 2.0}`:                         17,
		`{"distance_km": 1, "pending_orders": 0, "extra": 1}`:               13,
		`{"distance_km": 0, "pending_orders": 100000000000000000000000000}`: 2e26 + 10,
	}
	for body, want := range cases {
		rec := do(t, "POST", "/eta", body)
		var got map[string]any
		_ = json.Unmarshal(rec.Body.Bytes(), &got)
		if rec.Code != 200 || len(got) != 2 || got["model"] != "rule" ||
			math.Abs(got["minutes"].(float64)-want) > 1e-9*math.Max(1, want) {
			t.Errorf("%s: got %d %s, want minutes=%v", body, rec.Code, rec.Body, want)
		}
	}
}

// TestEtaInvalidReturns422: 음수·숫자 아님·누락·null·bool·NaN/Infinity·깨진 JSON·객체 아님은 422 다.
func TestEtaInvalidReturns422(t *testing.T) {
	bodies := []string{
		`{"distance_km": -1, "pending_orders": 0}`,
		`{"distance_km": -0.0001, "pending_orders": 0}`,
		`{"distance_km": 1, "pending_orders": -1}`,
		`{"distance_km": "abc", "pending_orders": 0}`,
		`{"distance_km": 1, "pending_orders": "many"}`,
		`{"distance_km": 1, "pending_orders": 1.5}`,
		`{"distance_km": 1}`,
		`{}`,
		`{"distance_km": null, "pending_orders": 0}`,
		`{"distance_km": true, "pending_orders": 0}`,
		`{"distance_km": 1, "pending_orders": false}`,
		`{"distance_km": NaN, "pending_orders": 0}`,
		`{"distance_km": Infinity, "pending_orders": 0}`,
		`{"distance_km": "inf", "pending_orders": 0}`,
		`{"distance_km": 1e308, "pending_orders": 0}`,
		`{"distance_km": 1,`,
		`[1, 2]`,
		`null`,
		``,
	}
	for _, body := range bodies {
		if rec := do(t, "POST", "/eta", body); rec.Code != 422 {
			t.Errorf("%s: got %d %s, want 422", body, rec.Code, rec.Body)
		}
	}
}

// TestEta422PointsToField: 422 의 detail[].loc 에 잘못된 필드가 ["body", 이름] 으로 들어 있다.
func TestEta422PointsToField(t *testing.T) {
	rec := do(t, "POST", "/eta", `{"distance_km": -1, "pending_orders": 0}`)
	var got struct {
		Detail []struct{ Loc []any } `json:"detail"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil || len(got.Detail) != 1 {
		t.Fatalf("got %d %s", rec.Code, rec.Body)
	}
	if loc := got.Detail[0].Loc; len(loc) != 2 || loc[0] != "body" || loc[1] != "distance_km" {
		t.Fatalf("loc = %v", loc)
	}
}

// TestWrongMethodReturns405: 허용되지 않은 HTTP 메서드는 405 다.
func TestWrongMethodReturns405(t *testing.T) {
	if rec := do(t, "GET", "/eta", ""); rec.Code != 405 {
		t.Errorf("GET /eta: %d", rec.Code)
	}
	if rec := do(t, "POST", "/health", ""); rec.Code != 405 {
		t.Errorf("POST /health: %d", rec.Code)
	}
}

// TestIndexPage: GET / 는 200, text/html 이고 모델 이름을 채우며 /docs 링크는 비운다.
func TestIndexPage(t *testing.T) {
	rec := do(t, "GET", "/", "")
	if rec.Code != 200 || !strings.HasPrefix(rec.Header().Get("Content-Type"), "text/html") {
		t.Fatalf("got %d %q", rec.Code, rec.Header().Get("Content-Type"))
	}
	if got := rec.Body.String(); got != `<p>모델: rule</p><nav></nav>` {
		t.Fatalf("body = %q", got)
	}
}

// TestIndexEscapesModelName: 모델 이름의 HTML 특수문자는 이스케이프한다.
func TestIndexEscapesModelName(t *testing.T) {
	if got := renderIndex("{{model}}", `<b>&`); got != "&lt;b&gt;&amp;" {
		t.Fatalf("got %q", got)
	}
}

// TestUnknownPathReturns404: 정의하지 않은 경로는 404 다.
func TestUnknownPathReturns404(t *testing.T) {
	if rec := do(t, "GET", "/docs", ""); rec.Code != 404 {
		t.Fatalf("GET /docs: %d", rec.Code)
	}
}
