package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"html"
	"io"
	"math"
	"net/http"
	"strconv"
	"strings"
)

// maxBodyBytes 는 /eta 요청 본문의 최대 크기다. 두 숫자 필드에는 넉넉하다.
const maxBodyBytes = 1 << 16

// Predictor 는 원래 단위(km, 건)의 입력으로 예상 시간(분)을 계산한다.
type Predictor func(distanceKm float64, pendingOrders float64) (float64, error)

// fieldError 는 FastAPI 의 422 응답 detail 항목과 같은 모양이다.
type fieldError struct {
	Type  string          `json:"type"`
	Loc   []any           `json:"loc"`
	Msg   string          `json:"msg"`
	Input json.RawMessage `json:"input,omitempty"`
}

type etaResponse struct {
	Minutes float64 `json:"minutes"`
	Model   string  `json:"model"`
}

// renderIndex 는 Python 서비스와 함께 쓰는 루트 페이지 템플릿(api/static/index.html)을 채운다.
// Go 서비스에는 /docs 가 없으므로 문서 링크는 비운다.
func renderIndex(tmpl, modelName string) string {
	return strings.NewReplacer(
		"{{model}}", html.EscapeString(modelName),
		"{{docs_link}}", "",
	).Replace(tmpl)
}

// newMux 는 api/app.py 와 같은 계약(GET /, GET /health, POST /eta)을 가진 라우터를 만든다.
// 메서드가 다르면 net/http 가 405 를, 없는 경로면 404 를 돌려준다.
func newMux(modelName, indexHTML string, predict Predictor) *http.ServeMux {
	mux := http.NewServeMux()
	page := []byte(renderIndex(indexHTML, modelName))
	mux.HandleFunc("GET /{$}", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/html; charset=utf-8")
		_, _ = w.Write(page)
	})
	mux.HandleFunc("GET /health", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, http.StatusOK, map[string]string{"status": "ok", "model": modelName})
	})
	mux.HandleFunc("POST /eta", func(w http.ResponseWriter, r *http.Request) {
		body, err := io.ReadAll(http.MaxBytesReader(w, r.Body, maxBodyBytes))
		if err != nil {
			writeErrors(w, []fieldError{{Type: "body_too_large", Loc: []any{"body"}, Msg: "Request body too large"}})
			return
		}
		distance, orders, errs := parseEtaRequest(body)
		if len(errs) > 0 {
			writeErrors(w, errs)
			return
		}
		minutes, err := predict(distance, orders)
		if err != nil {
			writeJSON(w, http.StatusInternalServerError, map[string]string{"detail": "prediction failed"})
			return
		}
		if math.IsNaN(minutes) || math.IsInf(minutes, 0) {
			// 너무 큰 입력으로 계산이 넘친 경우: null/inf 를 돌려주지 않는다
			writeErrors(w, []fieldError{{Type: "value_error", Loc: []any{"body"}, Msg: "Prediction is not a finite number"}})
			return
		}
		writeJSON(w, http.StatusOK, etaResponse{Minutes: minutes, Model: modelName})
	})
	return mux
}

// parseEtaRequest 는 본문을 검증해 (거리, 주문 수) 를 돌려준다.
// 규칙은 api/app.py 의 EtaRequest 와 같다:
//   - 본문은 JSON 객체여야 하고, 모르는 필드는 무시한다.
//   - distance_km: 숫자 또는 숫자 문자열, 0 이상.
//   - pending_orders: 정수(소수부가 0 인 실수, 정수 문자열 포함), 0 이상.
//
// 추가로 bool, NaN/Infinity, float32 범위를 넘는 값도 422 로 막는다.
func parseEtaRequest(body []byte) (float64, float64, []fieldError) {
	if !json.Valid(body) {
		return 0, 0, []fieldError{{Type: "json_invalid", Loc: []any{"body"}, Msg: "JSON decode error"}}
	}
	var obj map[string]json.RawMessage
	if err := json.Unmarshal(body, &obj); err != nil || obj == nil {
		return 0, 0, []fieldError{{Type: "model_attributes_type", Loc: []any{"body"},
			Msg: "Input should be a valid dictionary or object to extract fields from", Input: body}}
	}

	var errs []fieldError
	distance, e := parseNumber(obj, "distance_km", false)
	if e != nil {
		errs = append(errs, *e)
	}
	orders, e := parseNumber(obj, "pending_orders", true)
	if e != nil {
		errs = append(errs, *e)
	}
	return distance, orders, errs
}

// parseNumber 는 obj[name] 을 0 이상의 유한한 숫자로 읽는다. integer 면 정수만 받는다.
func parseNumber(obj map[string]json.RawMessage, name string, integer bool) (float64, *fieldError) {
	loc := []any{"body", name}
	raw, ok := obj[name]
	if !ok {
		return 0, &fieldError{Type: "missing", Loc: loc, Msg: "Field required"}
	}
	fail := func(typ, msg string) (float64, *fieldError) {
		return 0, &fieldError{Type: typ, Loc: loc, Msg: msg, Input: raw}
	}
	kind, parsing := "float", "Input should be a valid number"
	if integer {
		kind, parsing = "int", "Input should be a valid integer"
	}

	var text string
	switch t := bytes.TrimSpace(raw); {
	case len(t) > 0 && t[0] == '"':
		var s string
		if err := json.Unmarshal(t, &s); err != nil {
			return fail(kind+"_parsing", parsing+", unable to parse string as a number")
		}
		text = strings.TrimSpace(s)
	case len(t) > 0 && (t[0] == '-' || (t[0] >= '0' && t[0] <= '9')):
		text = string(t)
	default: // null, true/false, 객체, 배열
		return fail(kind+"_type", parsing)
	}

	v, err := strconv.ParseFloat(text, 64)
	if err != nil && !errors.Is(err, strconv.ErrRange) {
		return fail(kind+"_parsing", parsing+", unable to parse string as a number")
	}
	if err != nil || math.IsNaN(v) || math.IsInf(v, 0) {
		return fail("finite_number", "Input should be a finite number")
	}
	if integer && v != math.Trunc(v) {
		return fail("int_from_float", "Input should be a valid integer, got a number with a fractional part")
	}
	if v < 0 {
		return fail("greater_than_equal", "Input should be greater than or equal to 0")
	}
	if v > math.MaxFloat32 {
		// 모델 입력은 float32 이므로 그 범위를 넘으면 inf 가 된다
		return fail("less_than_equal", fmt.Sprintf("Input should be less than or equal to %g", math.MaxFloat32))
	}
	return v + 0, nil // -0.0 → 0
}

func writeErrors(w http.ResponseWriter, errs []fieldError) {
	writeJSON(w, http.StatusUnprocessableEntity, map[string][]fieldError{"detail": errs})
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}
