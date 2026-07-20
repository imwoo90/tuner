//! Axum task HTTP handlers and routers
//!
//! Implements routes and payload DTOs for creating, listing, cancelling, deleting, and resuming tasks.

//! 
//! ## Search Tags
//! #tasks

use axum::{
    extract::{Query, State},
    http::{StatusCode, HeaderMap},
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::webhook::api::server::ApiServerState;
use crate::webhook::api::tasks_models::*;

fn check_auth(state: &Arc<std::sync::Mutex<ApiServerState>>, headers: &HeaderMap) -> Result<(), axum::response::Response> {
    let token = { state.lock().unwrap().config.token.clone() };
    if !crate::webhook::api::files::verify_bearer(headers, &token) {
        return Err((StatusCode::UNAUTHORIZED, Json(StandardResponse {
            success: false,
            error: Some("Unauthorized".to_string()),
        })).into_response());
    }
    Ok(())
}

/// Handler for POST /tasks/create
pub async fn handle_task_create(
    State(state): State<Arc<std::sync::Mutex<ApiServerState>>>,
    headers: HeaderMap,
    Json(payload): Json<TaskCreateRequest>,
) -> impl IntoResponse {
    let token = { state.lock().unwrap().config.token.clone() };
    if !crate::webhook::api::files::verify_bearer(&headers, &token) {
        return (StatusCode::UNAUTHORIZED, Json(TaskCreateResponse {
            success: false,
            task_id: None,
            error: Some("Unauthorized".to_string()),
        })).into_response();
    }
    let hub = {
        let s = state.lock().unwrap();
        match &s.task_hub {
            Some(h) => h.clone(),
            None => return (StatusCode::SERVICE_UNAVAILABLE, Json(TaskCreateResponse {
                success: false,
                task_id: None,
                error: Some("Task system not initialized".to_string()),
            })).into_response(),
        }
    };

    if payload.prompt.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(TaskCreateResponse {
            success: false,
            task_id: None,
            error: Some("Missing prompt".to_string()),
        })).into_response();
    }

    let submit = payload.into_submit();

    match hub.submit(submit).await {
        Ok(task_id) => (
            StatusCode::OK,
            Json(TaskCreateResponse {
                success: true,
                task_id: Some(task_id),
                error: None,
            }),
        ).into_response(),
        Err(e) => (
            StatusCode::OK,
            Json(TaskCreateResponse {
                success: false,
                task_id: None,
                error: Some(e.to_string()),
            }),
        ).into_response(),
    }
}

/// Handler for POST /tasks/resume
pub async fn handle_task_resume(
    State(state): State<Arc<std::sync::Mutex<ApiServerState>>>,
    headers: HeaderMap,
    Json(payload): Json<TaskResumeRequest>,
) -> impl IntoResponse {
    if let Err(err) = check_auth(&state, &headers) {
        return err;
    }
    let hub = {
        let s = state.lock().unwrap();
        match &s.task_hub {
            Some(h) => h.clone(),
            None => return (StatusCode::SERVICE_UNAVAILABLE, Json(StandardResponse {
                success: false,
                error: Some("Task system not initialized".to_string()),
            })).into_response(),
        }
    };

    let entry = match hub.registry.get(&payload.task_id) {
        Some(e) => e,
        None => return (StatusCode::NOT_FOUND, Json(StandardResponse {
            success: false,
            error: Some("Task not found".to_string()),
        })).into_response(),
    };

    if entry.parent_agent != payload.parent_agent {
        return (StatusCode::FORBIDDEN, Json(StandardResponse {
            success: false,
            error: Some("Not authorized".to_string()),
        })).into_response();
    }

    match hub.resume(&payload.task_id, &payload.prompt).await {
        Ok(_) => (StatusCode::OK, Json(StandardResponse { success: true, error: None })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(StandardResponse {
            success: false,
            error: Some(e.to_string()),
        })).into_response(),
    }
}

/// Handler for POST /tasks/ask_parent
pub async fn handle_task_ask_parent(
    State(state): State<Arc<std::sync::Mutex<ApiServerState>>>,
    headers: HeaderMap,
    Json(payload): Json<TaskAskParentRequest>,
) -> impl IntoResponse {
    if let Err(err) = check_auth(&state, &headers) {
        return err;
    }
    let hub = {
        let s = state.lock().unwrap();
        match &s.task_hub {
            Some(h) => h.clone(),
            None => return (StatusCode::SERVICE_UNAVAILABLE, Json(TaskAskParentResponse {
                success: false,
                answer: String::new(),
                error: Some("Task system not initialized".to_string()),
            })).into_response(),
        }
    };

    match hub.forward_question(&payload.task_id, &payload.question).await {
        Ok(ans) => {
            let is_error = ans.starts_with("Error:");
            (
                StatusCode::OK,
                Json(TaskAskParentResponse {
                    success: !is_error,
                    answer: ans.clone(),
                    error: if is_error { Some(ans) } else { None },
                }),
            ).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(TaskAskParentResponse {
            success: false,
            answer: String::new(),
            error: Some(e.to_string()),
        })).into_response(),
    }
}

/// Handler for GET /tasks/list
pub async fn handle_task_list(
    State(state): State<Arc<std::sync::Mutex<ApiServerState>>>,
    headers: HeaderMap,
    Query(query): Query<TaskListQuery>,
) -> impl IntoResponse {
    if let Err(err) = check_auth(&state, &headers) {
        return err;
    }
    let hub = {
        let s = state.lock().unwrap();
        match &s.task_hub {
            Some(h) => h.clone(),
            None => return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                "tasks": []
            }))).into_response(),
        }
    };

    let tasks = hub.registry.list_all(None, query.from.as_deref());
    (StatusCode::OK, Json(serde_json::json!({ "tasks": tasks }))).into_response()
}

/// Handler for POST /tasks/cancel
pub async fn handle_task_cancel(
    State(state): State<Arc<std::sync::Mutex<ApiServerState>>>,
    headers: HeaderMap,
    Json(payload): Json<TaskCancelRequest>,
) -> impl IntoResponse {
    if let Err(err) = check_auth(&state, &headers) {
        return err;
    }
    let hub = {
        let s = state.lock().unwrap();
        match &s.task_hub {
            Some(h) => h.clone(),
            None => return (StatusCode::SERVICE_UNAVAILABLE, Json(StandardResponse {
                success: false,
                error: Some("Task system not initialized".to_string()),
            })).into_response(),
        }
    };

    let entry = match hub.registry.get(&payload.task_id) {
        Some(e) => e,
        None => return (StatusCode::NOT_FOUND, Json(StandardResponse {
            success: false,
            error: Some("Task not found".to_string()),
        })).into_response(),
    };

    if entry.parent_agent != payload.parent_agent {
        return (StatusCode::FORBIDDEN, Json(StandardResponse {
            success: false,
            error: Some("Not authorized".to_string()),
        })).into_response();
    }

    let success = hub.cancel(&payload.task_id).await;
    (StatusCode::OK, Json(StandardResponse {
        success,
        error: if success { None } else { Some("Task is not running".to_string()) },
    })).into_response()
}

/// Handler for POST /tasks/delete
pub async fn handle_task_delete(
    State(state): State<Arc<std::sync::Mutex<ApiServerState>>>,
    headers: HeaderMap,
    Json(payload): Json<TaskDeleteRequest>,
) -> impl IntoResponse {
    if let Err(err) = check_auth(&state, &headers) {
        return err;
    }
    let hub = {
        let s = state.lock().unwrap();
        match &s.task_hub {
            Some(h) => h.clone(),
            None => return (StatusCode::SERVICE_UNAVAILABLE, Json(StandardResponse {
                success: false,
                error: Some("Task system not initialized".to_string()),
            })).into_response(),
        }
    };

    let entry = match hub.registry.get(&payload.task_id) {
        Some(e) => e,
        None => return (StatusCode::NOT_FOUND, Json(StandardResponse {
            success: false,
            error: Some("Task not found".to_string()),
        })).into_response(),
    };

    if entry.parent_agent != payload.parent_agent {
        return (StatusCode::FORBIDDEN, Json(StandardResponse {
            success: false,
            error: Some("Not authorized".to_string()),
        })).into_response();
    }

    match hub.registry.delete(&payload.task_id) {
        Ok(true) => (StatusCode::OK, Json(StandardResponse { success: true, error: None })).into_response(),
        Ok(false) => (StatusCode::CONFLICT, Json(StandardResponse {
            success: false,
            error: Some("Task is still running or waiting".to_string()),
        })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(StandardResponse {
            success: false,
            error: Some(e.to_string()),
        })).into_response(),
    }
}

