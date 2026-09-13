//! `/mute` エンドポイント。マスターミュート状態の変更(`POST`)と
//! 現在の状態確認(`GET`)を扱う。

use std::sync::atomic::Ordering;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::ApiState;

/// `POST /mute` のリクエストボディ。
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct MuteRequest {
    /// 更新後のミュート状態
    pub is_muted: bool,
}

/// `/mute` のレスポンスボディ。`POST`は更新後、`GET`は現在の状態を返す。
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct MuteResponse {
    /// ミュート状態
    pub is_muted: bool,
}

/// マスターミュート状態を変更する。
#[utoipa::path(
    post,
    path = "/mute",
    request_body = MuteRequest,
    responses(
        (status = 200, description = "ミュート状態を更新した", body = MuteResponse)
    ),
    tag = "mute"
)]
pub async fn mute(
    State(state): State<ApiState>,
    Json(request): Json<MuteRequest>,
) -> Json<MuteResponse> {
    state.is_muted.store(request.is_muted, Ordering::Relaxed);
    Json(MuteResponse {
        is_muted: request.is_muted,
    })
}

/// マスターミュートの現在の状態を取得する。
#[utoipa::path(
    get,
    path = "/mute",
    responses(
        (status = 200, description = "現在のミュート状態", body = MuteResponse)
    ),
    tag = "mute"
)]
pub async fn get_mute(State(state): State<ApiState>) -> Json<MuteResponse> {
    Json(MuteResponse {
        is_muted: state.is_muted.load(Ordering::Relaxed),
    })
}
