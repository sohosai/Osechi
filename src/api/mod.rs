//! donguri(音響卓側システム)など外部システムから操作を受け付けるHTTP API。
//!
//! v1では `/mute`(状態の変更・確認)のみを実装する(`docs/mute-api.md` 参照)。
//! OpenAPIスキーマは起動後 `http://<host>:<port>/api-docs/openapi.json`、
//! Swagger UIは `http://<host>:<port>/swagger-ui` で確認できる。
//!
//! listenポートは環境変数 `OSECHI_API_PORT` で上書きできる(未指定時は
//! [`DEFAULT_PORT`])。

mod mute;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use axum::routing::post;
use tokio::net::TcpListener;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use mute::{MuteRequest, MuteResponse};

/// listenポートの環境変数名。
const PORT_ENV_VAR: &str = "OSECHI_API_PORT";
/// 環境変数が未指定・不正な場合に使うデフォルトのlistenポート。
const DEFAULT_PORT: u16 = 7878;

/// ハンドラ間で共有する状態。
#[derive(Clone)]
struct ApiState {
    is_muted: Arc<AtomicBool>,
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Osechi External API",
        description = "donguriなど外部システムからOsechiを操作するためのAPI。",
        version = "0.1.0"
    ),
    paths(mute::mute, mute::get_mute),
    components(schemas(MuteRequest, MuteResponse))
)]
struct ApiDoc;

/// `OSECHI_API_PORT` の値、無ければ [`DEFAULT_PORT`] を返す。
/// 値が数値としてパースできない場合も [`DEFAULT_PORT`] にフォールバックする。
fn resolve_port() -> u16 {
    std::env::var(PORT_ENV_VAR)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

fn build_router(is_muted: Arc<AtomicBool>) -> Router {
    let state = ApiState { is_muted };

    Router::new()
        .route("/mute", post(mute::mute).get(mute::get_mute))
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .with_state(state)
}

/// APIサーバーを専用スレッド上のTokioランタイムで起動する。
///
/// eframeのメインループ(同期・シングルスレッドの毎フレーム更新)を
/// ブロックしないよう、独立したOSスレッド上でTokioランタイムを立てて
/// サーバーを動かす。ポートのbindに失敗した場合(使用中など)は
/// エラーをログに出すのみで、アプリ本体の起動は妨げない。
pub fn spawn_mute_server(is_muted: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(e) => {
                tracing::error!("failed to start API server runtime: {e}");
                return;
            }
        };

        runtime.block_on(async move {
            let port = resolve_port();
            let listener = match TcpListener::bind(("0.0.0.0", port)).await {
                Ok(listener) => listener,
                Err(e) => {
                    tracing::error!("failed to bind API server on port {port}: {e}");
                    return;
                }
            };

            tracing::info!(
                "API server listening on 0.0.0.0:{port} (Swagger UI: http://localhost:{port}/swagger-ui, OpenAPI: http://localhost:{port}/api-docs/openapi.json)"
            );

            let router = build_router(is_muted);
            if let Err(e) = axum::serve(listener, router).await {
                tracing::error!("API server stopped unexpectedly: {e}");
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn mute_endpoint_updates_shared_state_and_echoes_it_back() {
        let is_muted = Arc::new(AtomicBool::new(false));
        let router = build_router(Arc::clone(&is_muted));

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mute")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"is_muted":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(is_muted.load(std::sync::atomic::Ordering::Relaxed));

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, r#"{"is_muted":true}"#.as_bytes());
    }

    #[tokio::test]
    async fn get_mute_returns_current_state_without_changing_it() {
        let is_muted = Arc::new(AtomicBool::new(true));
        let router = build_router(Arc::clone(&is_muted));

        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/mute")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        // GETは状態を変更しない
        assert!(is_muted.load(std::sync::atomic::Ordering::Relaxed));

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, r#"{"is_muted":true}"#.as_bytes());
    }

    #[tokio::test]
    async fn mute_endpoint_rejects_invalid_body() {
        let router = build_router(Arc::new(AtomicBool::new(false)));

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mute")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"is_muted": "yes"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn openapi_schema_is_served() {
        let router = build_router(Arc::new(AtomicBool::new(false)));

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api-docs/openapi.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["paths"]["/mute"]["post"]["operationId"], "mute");
    }

    #[tokio::test]
    async fn swagger_ui_is_served() {
        let router = build_router(Arc::new(AtomicBool::new(false)));

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/swagger-ui")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // swagger-uiは末尾スラッシュ無しのアクセスを `/swagger-ui/` へ
        // リダイレクトする。
        assert!(response.status().is_redirection() || response.status().is_success());
    }

    #[test]
    fn resolve_port_falls_back_to_default_when_unset() {
        // 環境変数はプロセス全体で共有されるため、テストが並行実行されても
        // 他のテストに影響しない専用の変数名を使う。
        // SAFETY: このプロセスの他のテストは "OSECHI_API_PORT" を参照しない。
        unsafe {
            std::env::remove_var(PORT_ENV_VAR);
        }
        assert_eq!(resolve_port(), DEFAULT_PORT);
    }
}
