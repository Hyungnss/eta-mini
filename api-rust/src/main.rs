//! eta-api-rust: api/app.py 와 같은 계약(GET /, GET /health, POST /eta)을 ONNX Runtime 으로 서빙한다.
//!
//! 환경 변수
//!   PORT        듣는 포트. 기본 8080
//!   MODEL_PATH  ONNX 모델 경로. 기본 artifacts/model_sklearn.onnx
//!   ORT_LIB     libonnxruntime.so 경로. 기본 /usr/local/lib/libonnxruntime.so
//!   MODEL_NAME  /, /health, /eta 응답의 model 값. 기본 onnx
//!   INDEX_PATH  루트 페이지 템플릿 (Python·Go 와 공유). 기본 api/static/index.html

mod handler;

use std::sync::{Arc, Mutex};

use ort::{environment::Environment, session::Session, value::Tensor};

use handler::Predictor;

fn getenv(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

/// 모델(입력 "X" [1, 2] float32 → 출력 "variable" [1, 1])을 불러온다.
/// 정규화는 모델 안에 있으므로 원래 단위의 값을 그대로 넣는다.
fn onnx_predictor(env: &Environment, model_path: &str) -> Result<Predictor, String> {
    let session = Session::builder(env)
        .and_then(|mut b| b.commit_from_file(model_path))
        .map_err(|e| format!("모델 불러오기 실패 ({model_path}): {e}"))?;
    let session = Mutex::new(session);

    Ok(Arc::new(move |distance_km: f64, pending_orders: f64| {
        let x = Tensor::from_array(([1usize, 2], vec![distance_km as f32, pending_orders as f32]))
            .map_err(|e| e.to_string())?;
        let mut session = session.lock().map_err(|e| e.to_string())?;
        let outputs = session
            .run(ort::inputs!["X" => x])
            .map_err(|e| e.to_string())?;
        let (_, data) = outputs["variable"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        data.first()
            .map(|&y| y as f64)
            .ok_or_else(|| "빈 출력".to_owned())
    }))
}

#[tokio::main]
async fn main() {
    let ort_lib = getenv("ORT_LIB", "/usr/local/lib/libonnxruntime.so");
    let env = match ort::init_from(&ort_lib)
        .map_err(|e| e.to_string())
        .and_then(|b| b.build().map_err(|e| e.to_string()))
    {
        Ok(env) => env,
        Err(e) => {
            eprintln!("ONNX Runtime 초기화 실패 ({ort_lib}): {e}");
            std::process::exit(1);
        }
    };

    let predict = match onnx_predictor(&env, &getenv("MODEL_PATH", "artifacts/model_sklearn.onnx"))
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let index_path = getenv("INDEX_PATH", "api/static/index.html");
    let index_html = std::fs::read_to_string(&index_path).unwrap_or_else(|e| {
        eprintln!("루트 페이지 템플릿 읽기 실패 ({index_path}): {e}");
        std::process::exit(1);
    });

    let addr = format!("0.0.0.0:{}", getenv("PORT", "8080"));
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("{addr} 에서 듣기 실패: {e}");
            std::process::exit(1);
        });
    eprintln!("ONNX Runtime {ort_lib}, listening on {addr}");
    let app = handler::router(&getenv("MODEL_NAME", "onnx"), &index_html, predict);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

/// Ctrl+C(SIGINT) 또는 docker stop(SIGTERM) 을 받으면 끝난다. 컨테이너의 PID 1 은 SIGTERM 을 직접 처리해야 한다.
async fn shutdown_signal() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGTERM 핸들러 등록 실패");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
