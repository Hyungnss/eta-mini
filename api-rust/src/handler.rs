//! api/app.py 와 같은 계약(GET /, GET /health, POST /eta)을 가진 라우터와 입력 검증.
//!
//! axum 의 `Json` 추출기는 형식 오류에 400·415 를 돌려주므로 쓰지 않는다.
//! 본문을 바이트로 받아 직접 파싱해 모든 입력 오류를 422 로 맞춘다.

use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use serde_json::{Map, Value, json};

/// /eta 요청 본문의 최대 크기. 두 숫자 필드에는 넉넉하다.
pub const MAX_BODY_BYTES: usize = 1 << 16;

/// 원래 단위(km, 건)의 입력으로 예상 시간(분)을 계산한다.
pub type Predictor = Arc<dyn Fn(f64, f64) -> Result<f64, String> + Send + Sync>;

#[derive(Clone)]
struct AppState {
    model_name: Arc<str>,
    predict: Predictor,
}

/// FastAPI 의 422 응답 detail 항목과 같은 모양.
#[derive(Debug, Serialize)]
pub struct FieldError {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub loc: Vec<&'static str>,
    pub msg: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
}

impl FieldError {
    fn body(kind: &'static str, msg: &str) -> Self {
        Self {
            kind,
            loc: vec!["body"],
            msg: msg.to_owned(),
            input: None,
        }
    }
}

#[derive(Serialize)]
struct EtaResponse<'a> {
    minutes: f64,
    model: &'a str,
}

/// Python·Go 서비스와 함께 쓰는 루트 페이지 템플릿(api/static/index.html)을 채운다.
/// Rust 서비스에는 /docs 가 없으므로 문서 링크는 비운다.
pub fn render_index(tmpl: &str, model_name: &str) -> String {
    tmpl.replace("{{model}}", &escape_html(model_name))
        .replace("{{docs_link}}", "")
}

/// Go 의 html.EscapeString 과 같은 다섯 문자를 이스케이프한다.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&#34;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// api/app.py 와 같은 계약(GET /, GET /health, POST /eta)을 가진 라우터를 만든다.
/// 메서드가 다르면 axum 이 405 를, 없는 경로면 404 를 돌려준다.
pub fn router(model_name: &str, index_html: &str, predict: Predictor) -> Router {
    let state = AppState {
        model_name: model_name.into(),
        predict,
    };
    let page = Html(render_index(index_html, model_name));
    Router::new()
        .route("/", get(move || async move { page }))
        .route("/health", get(health))
        .route("/eta", post(eta))
        .with_state(state)
}

async fn health(State(s): State<AppState>) -> Response {
    Json(json!({"status": "ok", "model": &*s.model_name})).into_response()
}

async fn eta(State(s): State<AppState>, body: Body) -> Response {
    let Ok(bytes) = to_bytes(body, MAX_BODY_BYTES).await else {
        return errors(vec![FieldError::body(
            "body_too_large",
            "Request body too large",
        )]);
    };
    let (distance, orders) = match parse_eta_request(&bytes) {
        Ok(v) => v,
        Err(errs) => return errors(errs),
    };
    let minutes = match (s.predict)(distance, orders) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("prediction failed: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"detail": "prediction failed"})),
            )
                .into_response();
        }
    };
    if !minutes.is_finite() {
        // 너무 큰 입력으로 계산이 넘친 경우: null/inf 를 돌려주지 않는다
        return errors(vec![FieldError::body(
            "value_error",
            "Prediction is not a finite number",
        )]);
    }
    Json(EtaResponse {
        minutes,
        model: &s.model_name,
    })
    .into_response()
}

fn errors(errs: Vec<FieldError>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"detail": errs})),
    )
        .into_response()
}

/// 본문을 검증해 (거리, 주문 수) 를 돌려준다.
/// 규칙은 api/app.py 의 EtaRequest(그리고 api-go)와 같다:
///   - 본문은 JSON 객체여야 하고, 모르는 필드는 무시한다.
///   - distance_km: 숫자 또는 숫자 문자열, 0 이상.
///   - pending_orders: 정수(소수부가 0 인 실수, 정수 문자열 포함), 0 이상.
///
/// 추가로 bool, NaN/Infinity, float32 범위를 넘는 값도 422 로 막는다.
pub fn parse_eta_request(body: &[u8]) -> Result<(f64, f64), Vec<FieldError>> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| vec![FieldError::body("json_invalid", "JSON decode error")])?;
    let Value::Object(obj) = value else {
        return Err(vec![FieldError {
            input: Some(value),
            ..FieldError::body(
                "model_attributes_type",
                "Input should be a valid dictionary or object to extract fields from",
            )
        }]);
    };

    let distance = parse_number(&obj, "distance_km", false);
    let orders = parse_number(&obj, "pending_orders", true);
    match (distance, orders) {
        (Ok(d), Ok(o)) => Ok((d, o)),
        (d, o) => Err([d.err(), o.err()].into_iter().flatten().collect()),
    }
}

/// obj[name] 을 0 이상의 유한한 숫자로 읽는다. integer 면 정수만 받는다.
fn parse_number(
    obj: &Map<String, Value>,
    name: &'static str,
    integer: bool,
) -> Result<f64, FieldError> {
    let loc = vec!["body", name];
    let Some(raw) = obj.get(name) else {
        return Err(FieldError {
            kind: "missing",
            loc,
            msg: "Field required".into(),
            input: None,
        });
    };
    let fail = |kind: &'static str, msg: String| FieldError {
        kind,
        loc: loc.clone(),
        msg,
        input: Some(raw.clone()),
    };
    let (type_err, parse_err, parsing) = if integer {
        ("int_type", "int_parsing", "Input should be a valid integer")
    } else {
        (
            "float_type",
            "float_parsing",
            "Input should be a valid number",
        )
    };

    let v = match raw {
        Value::Number(n) => n.as_f64().ok_or_else(|| fail(type_err, parsing.into()))?,
        Value::String(s) => s.trim().parse::<f64>().map_err(|_| {
            fail(
                parse_err,
                format!("{parsing}, unable to parse string as a number"),
            )
        })?,
        _ => return Err(fail(type_err, parsing.into())), // null, true/false, 객체, 배열
    };

    if !v.is_finite() {
        return Err(fail(
            "finite_number",
            "Input should be a finite number".into(),
        ));
    }
    if integer && v != v.trunc() {
        return Err(fail(
            "int_from_float",
            "Input should be a valid integer, got a number with a fractional part".into(),
        ));
    }
    if v < 0.0 {
        return Err(fail(
            "greater_than_equal",
            "Input should be greater than or equal to 0".into(),
        ));
    }
    if v > f32::MAX as f64 {
        // 모델 입력은 float32 이므로 그 범위를 넘으면 inf 가 된다
        return Err(fail(
            "less_than_equal",
            format!("Input should be less than or equal to {:e}", f32::MAX),
        ));
    }
    Ok(v + 0.0) // -0.0 → 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// api/predictors/rule.py 와 같은 식. 계약(검증·응답 형식) 확인용으로 모델 대신 쓴다.
    fn rule() -> Predictor {
        Arc::new(|d, p| Ok(10.0 + 3.0 * d + 2.0 * p))
    }

    /// 테스트용 루트 페이지 템플릿. 자리표시자는 api/static/index.html 과 같다.
    const TEST_INDEX: &str = "<p>모델: {{model}}</p><nav>{{docs_link}}</nav>";

    async fn call(
        method: &str,
        path: &str,
        body: &str,
        content_type: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(ct) = content_type {
            req = req.header("content-type", ct);
        }
        let res = router("rule", TEST_INDEX, rule())
            .oneshot(req.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn post_eta(body: &str) -> (StatusCode, Value) {
        call("POST", "/eta", body, Some("application/json")).await
    }

    /// /health 는 200 과 status=ok, model 이름을 돌려준다.
    #[tokio::test]
    async fn health_ok() {
        let (status, body) = call("GET", "/health", "", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"status": "ok", "model": "rule"}));
    }

    /// 정상·경계·관대한 입력에 대해 200 과 minutes, model 두 필드만 돌려준다.
    #[tokio::test]
    async fn eta_known_values() {
        let cases = [
            (r#"{"distance_km": 0, "pending_orders": 0}"#, 10.0),
            (r#"{"distance_km": 2.5, "pending_orders": 3}"#, 23.5),
            (r#"{"distance_km": 1000, "pending_orders": 10000}"#, 23010.0),
            (r#"{"distance_km": -0.0, "pending_orders": 0}"#, 10.0),
            (r#"{"distance_km": "1.5", "pending_orders": "2"}"#, 18.5),
            (r#"{"distance_km": 1, "pending_orders": 2.0}"#, 17.0),
            (
                r#"{"distance_km": 1, "pending_orders": 0, "extra": 1}"#,
                13.0,
            ),
            (
                r#"{"distance_km": 0, "pending_orders": 100000000000000000000000000}"#,
                2e26 + 10.0,
            ),
        ];
        for (body, want) in cases {
            let (status, got) = post_eta(body).await;
            assert_eq!(status, StatusCode::OK, "{body}: {got}");
            let obj = got.as_object().unwrap();
            assert_eq!(obj.len(), 2, "{body}: {got}");
            assert_eq!(got["model"], "rule");
            let minutes = got["minutes"].as_f64().unwrap();
            assert!(
                (minutes - want).abs() <= 1e-9 * want.max(1.0),
                "{body}: got {minutes}, want {want}"
            );
        }
    }

    /// Content-Type 이 없거나 JSON 이 아니어도 본문이 올바르면 처리한다 (415 를 내지 않는다).
    #[tokio::test]
    async fn eta_ignores_content_type() {
        let body = r#"{"distance_km": 1, "pending_orders": 1}"#;
        for ct in [None, Some("text/plain")] {
            let (status, got) = call("POST", "/eta", body, ct).await;
            assert_eq!(status, StatusCode::OK, "{ct:?}: {got}");
            assert_eq!(got["minutes"], 15.0);
        }
    }

    /// 음수·숫자 아님·누락·null·bool·NaN/Infinity·깨진 JSON·객체 아님은 422 다.
    #[tokio::test]
    async fn eta_invalid_returns_422() {
        let bodies = [
            r#"{"distance_km": -1, "pending_orders": 0}"#,
            r#"{"distance_km": -0.0001, "pending_orders": 0}"#,
            r#"{"distance_km": 1, "pending_orders": -1}"#,
            r#"{"distance_km": "abc", "pending_orders": 0}"#,
            r#"{"distance_km": 1, "pending_orders": "many"}"#,
            r#"{"distance_km": 1, "pending_orders": 1.5}"#,
            r#"{"distance_km": 1}"#,
            r#"{}"#,
            r#"{"distance_km": null, "pending_orders": 0}"#,
            r#"{"distance_km": true, "pending_orders": 0}"#,
            r#"{"distance_km": 1, "pending_orders": false}"#,
            r#"{"distance_km": [1], "pending_orders": 0}"#,
            r#"{"distance_km": NaN, "pending_orders": 0}"#,
            r#"{"distance_km": Infinity, "pending_orders": 0}"#,
            r#"{"distance_km": "inf", "pending_orders": 0}"#,
            r#"{"distance_km": "NaN", "pending_orders": 0}"#,
            r#"{"distance_km": 1e308, "pending_orders": 0}"#,
            r#"{"distance_km": 1e400, "pending_orders": 0}"#,
            r#"{"distance_km": 1,"#,
            r#"[1, 2]"#,
            r#"null"#,
            r#""#,
        ];
        for body in bodies {
            let (status, got) = post_eta(body).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}: {got}");
            assert!(
                got["detail"].as_array().is_some_and(|d| !d.is_empty()),
                "{body}: {got}"
            );
        }
    }

    /// 422 detail 은 FastAPI 처럼 필드별 type, loc 을 담고, 두 필드가 모두 틀리면 둘 다 알린다.
    #[tokio::test]
    async fn eta_422_detail_shape() {
        let (status, got) = post_eta(r#"{"distance_km": -1}"#).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(got["detail"][0]["type"], "greater_than_equal");
        assert_eq!(got["detail"][0]["loc"], json!(["body", "distance_km"]));
        assert_eq!(got["detail"][1]["type"], "missing");
        assert_eq!(got["detail"][1]["loc"], json!(["body", "pending_orders"]));
    }

    /// 본문이 최대 크기를 넘으면 413 이 아니라 422 다.
    #[tokio::test]
    async fn eta_body_too_large_returns_422() {
        let body = format!(
            r#"{{"distance_km": 1, "pending_orders": 0, "pad": "{}"}}"#,
            "x".repeat(MAX_BODY_BYTES)
        );
        let (status, got) = post_eta(&body).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(got["detail"][0]["type"], "body_too_large");
    }

    /// 지원하지 않는 메서드는 405, 없는 경로는 404 다.
    #[tokio::test]
    async fn wrong_method_or_path() {
        assert_eq!(
            call("GET", "/eta", "", None).await.0,
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            call("POST", "/health", "", None).await.0,
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            call("GET", "/nope", "", None).await.0,
            StatusCode::NOT_FOUND
        );
    }

    /// GET / 는 200, text/html 이고 모델 이름을 채우며 /docs 링크는 비운다.
    #[tokio::test]
    async fn index_page() {
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let res = router("rule", TEST_INDEX, rule())
            .oneshot(req)
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let ct = res.headers()["content-type"].to_str().unwrap().to_owned();
        assert!(ct.starts_with("text/html"), "content-type = {ct}");
        let body = res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], "<p>모델: rule</p><nav></nav>".as_bytes());
    }

    /// 모델 이름의 HTML 특수문자는 이스케이프한다.
    #[test]
    fn index_escapes_model_name() {
        assert_eq!(
            render_index("{{model}}", r#"<b>&"'"#),
            "&lt;b&gt;&amp;&#34;&#39;"
        );
    }

    /// Rust 서비스에는 문서 페이지가 없으므로 /docs 는 404 다.
    #[tokio::test]
    async fn docs_returns_404() {
        assert_eq!(
            call("GET", "/docs", "", None).await.0,
            StatusCode::NOT_FOUND
        );
    }
}
