use axum::{
    extract::{Path, Multipart},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use serde_json::json;
use sha2::{Sha256, Digest};
use std::io::Read;

use super::super::ports::image_store::ImageStorePort;
use super::super::use_cases::upload_image::UploadImageUseCase;

// ADR-2026-05-19-0721
pub fn routes(image_store: ImageStorePort) -> Router {
    let upload_image_use_case = UploadImageUseCase::new(image_store);
    Router::new()
        .route("/api/v1/images", post(upload_image_handler))
        .with_state(upload_image_use_case)
}

async fn upload_image_handler(
    mut multipart: Multipart,
    state: axum::extract::State<UploadImageUseCase>,
) -> impl IntoResponse {
    let chunk_size_limit = 5 * 1024 * 1024; // 5 MiB limit per field
    let mut data = Vec::new();
    let mut content_type = String::new();
    let mut sha256_hasher = Sha256::new();

    while let Some(field) = multipart.next_field().await.unwrap() {
        if let Some(file_name) = field.file_name() {
            let content_disposition = field.content_disposition().unwrap();
            content_type = content_disposition.get("Content-Type").unwrap_or("").to_string();
            
            let mut stream = field;
            while let Ok(Some(bytes)) = stream.chunk().await {
                if data.len() + bytes.len() > chunk_size_limit {
                    return (StatusCode::PAYLOAD_TOO_LARGE, "File size exceeds 5 MiB limit").into_response();
                }
                sha256_hasher.update(&bytes);
                data.extend_from_slice(&bytes);
            }

            let sha256 = hex::encode(sha256_hasher.finalize());
            
            match state.upload_image(data.clone(), content_type.clone()).await {
                Ok(_) => return (StatusCode::CREATED, Json(json!({ "sha256": sha256, "content_type": content_type, "byte_size": data.len() }))).into_response(),
                Err(e) => match e {
                    crate::core::errors::ImageStoreError::TooLarge => return (StatusCode::PAYLOAD_TOO_LARGE, "File size exceeds 5 MiB limit").into_response(),
                    crate::core::errors::ImageStoreError::UnsupportedMediaType => return (StatusCode::UNSUPPORTED_MEDIA_TYPE, "Unsupported media type").into_response(),
                    _ => return (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response(),
                }
            }
        } else {
            return (StatusCode::BAD_REQUEST, "No file found in multipart form data").into_response();
        }
    }

    (StatusCode::BAD_REQUEST, "Failed to process multipart form data").into_response()
}