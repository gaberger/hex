use actix_web::{web, HttpResponse, Responder};
use serde::Serialize;
use chrono::Utc;

#[derive(Serialize)]
struct WorkplanStatus {
    current_phase: String,
    task: String,
    progress: (u32, u32),
    last_update_timestamp: String,
}

async fn get_workplan_status(id: web::Path<i32>) -> impl Responder {
    // Mock data for demonstration purposes
    let workplan_status = WorkplanStatus {
        current_phase: "Design".to_string(),
        task: "Wireframing".to_string(),
        progress: (5, 10),
        last_update_timestamp: Utc::now().to_rfc3339(),
    };

    HttpResponse::Ok().json(workplan_status)
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/api/workplan/{id}/status", web::get().to(get_workplan_status));
}