// eta-api-go: api/app.py 와 같은 계약(GET /, GET /health, POST /eta)을 ONNX Runtime 으로 서빙한다.
//
// 환경 변수
//
//	PORT          듣는 포트. 기본 8080
//	MODEL_PATH    ONNX 모델 경로. 기본 artifacts/model_sklearn.onnx
//	ORT_LIB_PATH  libonnxruntime.so 경로. 기본 /usr/local/lib/libonnxruntime.so
//	MODEL_NAME    /health, /eta 응답의 model 값. 기본 onnx
//	INDEX_PATH    루트 페이지 템플릿 (Python 과 공유). 기본 api/static/index.html
package main

import (
	"fmt"
	"log"
	"net/http"
	"os"
	"time"

	ort "github.com/yalue/onnxruntime_go"
)

func getenv(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}

// newOnnxPredictor 는 모델(입력 "X" [1, 2] float32 → 출력 "variable" [1, 1])을 불러온다.
// 정규화는 모델 안에 있으므로 원래 단위의 값을 그대로 넣는다.
// DynamicAdvancedSession 은 동시에 Run 해도 안전하고, 텐서는 요청마다 만든다.
func newOnnxPredictor(modelPath string) (Predictor, func(), error) {
	session, err := ort.NewDynamicAdvancedSession(modelPath, []string{"X"}, []string{"variable"}, nil)
	if err != nil {
		return nil, nil, fmt.Errorf("모델 불러오기 실패 (%s): %w", modelPath, err)
	}
	predict := func(distanceKm, pendingOrders float64) (float64, error) {
		x, err := ort.NewTensor(ort.NewShape(1, 2), []float32{float32(distanceKm), float32(pendingOrders)})
		if err != nil {
			return 0, err
		}
		defer x.Destroy()
		y, err := ort.NewEmptyTensor[float32](ort.NewShape(1, 1))
		if err != nil {
			return 0, err
		}
		defer y.Destroy()
		if err := session.Run([]ort.Value{x}, []ort.Value{y}); err != nil {
			return 0, err
		}
		return float64(y.GetData()[0]), nil
	}
	return predict, func() { session.Destroy() }, nil
}

func main() {
	ort.SetSharedLibraryPath(getenv("ORT_LIB_PATH", "/usr/local/lib/libonnxruntime.so"))
	if err := ort.InitializeEnvironment(); err != nil {
		log.Fatalf("ONNX Runtime 초기화 실패: %v", err)
	}
	defer ort.DestroyEnvironment()

	predict, closeSession, err := newOnnxPredictor(getenv("MODEL_PATH", "artifacts/model_sklearn.onnx"))
	if err != nil {
		log.Fatal(err)
	}
	defer closeSession()

	indexPath := getenv("INDEX_PATH", "api/static/index.html")
	indexHTML, err := os.ReadFile(indexPath)
	if err != nil {
		log.Fatalf("루트 페이지 템플릿 읽기 실패: %v", err)
	}

	addr := ":" + getenv("PORT", "8080")
	srv := &http.Server{
		Addr:              addr,
		Handler:           newMux(getenv("MODEL_NAME", "onnx"), string(indexHTML), predict),
		ReadHeaderTimeout: 5 * time.Second,
	}
	log.Printf("ONNX Runtime %s, listening on %s", ort.GetVersion(), addr)
	log.Fatal(srv.ListenAndServe())
}
