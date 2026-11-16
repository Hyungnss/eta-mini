# Rust(axum) + ONNX Runtime 으로 api/app.py 와 같은 계약을 서빙한다.
# 빌드 컨텍스트는 저장소 루트다:
#   docker build -f api-rust/Dockerfile -t eta-api-rust .
#   docker run --rm -p 8082:8080 eta-api-rust

# ── 1. Rust 바이너리 (ort 의 load-dynamic: 실행할 때 ORT_LIB 의 libonnxruntime.so 를 dlopen 한다) ──
FROM rust:1.100-bookworm AS build
WORKDIR /src
# 의존성만 먼저 빌드해 레이어 캐시를 쓴다 (소스만 바뀌면 이 단계는 재사용된다)
COPY api-rust/Cargo.toml api-rust/Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
 && cargo build --release --locked \
 && rm -rf src
COPY api-rust/src ./src
RUN touch src/main.rs \
 && cargo test --release --locked \
 && cargo build --release --locked \
 && cp target/release/eta-api /eta-api

# ── 2. ONNX Runtime 공유 라이브러리: 버전은 uv.lock 의 onnxruntime 과 같게 ──
FROM debian:bookworm-slim AS ort
ARG TARGETARCH
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
COPY uv.lock /tmp/uv.lock
RUN set -eux; \
    ORT_VERSION="$(awk '/^name = "onnxruntime"$/ { getline; gsub(/^version = "|"$/, ""); print; exit }' /tmp/uv.lock)"; \
    test -n "$ORT_VERSION"; \
    case "$TARGETARCH" in \
      amd64) ORT_ARCH=x64 ;; \
      arm64) ORT_ARCH=aarch64 ;; \
      *) echo "지원하지 않는 TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}.tgz" \
      | tar -xz -C /tmp; \
    mkdir -p /out; \
    cp -L "/tmp/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}/lib/libonnxruntime.so.${ORT_VERSION}" /out/libonnxruntime.so; \
    echo "$ORT_VERSION" > /out/ORT_VERSION

# ── 3. 실행 이미지 ──
FROM debian:bookworm-slim
WORKDIR /app
COPY --from=ort /out/libonnxruntime.so /usr/local/lib/libonnxruntime.so
COPY --from=ort /out/ORT_VERSION /usr/local/lib/ORT_VERSION
COPY --from=build /eta-api /usr/local/bin/eta-api
COPY artifacts/model_sklearn.onnx /app/artifacts/model_sklearn.onnx
COPY api/static/index.html /app/api/static/index.html

ENV PORT=8080 \
    MODEL_PATH=/app/artifacts/model_sklearn.onnx \
    ORT_LIB=/usr/local/lib/libonnxruntime.so \
    MODEL_NAME=onnx \
    INDEX_PATH=/app/api/static/index.html

USER nobody
EXPOSE 8080
CMD ["eta-api"]
