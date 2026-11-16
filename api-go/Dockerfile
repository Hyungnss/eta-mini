# Go + ONNX Runtime 으로 api/app.py 와 같은 계약을 서빙한다.
# 빌드 컨텍스트는 저장소 루트다:
#   docker build -f api-go/Dockerfile -t eta-api-go .
#   docker run --rm -p 8081:8080 eta-api-go

# ── 1. Go 바이너리 (onnxruntime_go 는 cgo 로 libonnxruntime.so 를 dlopen 한다) ──
FROM golang:1.27-bookworm AS build
WORKDIR /src
COPY api-go/go.mod api-go/go.sum ./
RUN go mod download
COPY api-go/*.go ./
RUN go test ./... && CGO_ENABLED=1 go build -trimpath -ldflags="-s -w" -o /out/eta-api .

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
COPY --from=build /out/eta-api /usr/local/bin/eta-api
COPY artifacts/model_sklearn.onnx /app/artifacts/model_sklearn.onnx
# 루트 페이지 템플릿은 Python 서비스(api/app.py)와 같은 파일을 쓴다
COPY api/static/index.html /app/api/static/index.html

ENV PORT=8080 \
    MODEL_PATH=/app/artifacts/model_sklearn.onnx \
    ORT_LIB_PATH=/usr/local/lib/libonnxruntime.so \
    MODEL_NAME=onnx \
    INDEX_PATH=/app/api/static/index.html

USER nobody
EXPOSE 8080
CMD ["eta-api"]
