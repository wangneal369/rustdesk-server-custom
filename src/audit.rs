use crate::{common::get_arg, database::{AuditWrite, Database}};
use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use hbb_common::{anyhow::ensure, log, tokio, ResultType};
use serde_json::Value;
use std::net::SocketAddr;

#[derive(Clone)]
struct AuditState {
    db: Database,
    token: String,
}

pub(crate) async fn run(db: Database) -> ResultType<()> {
    let bind = get_arg("AUDIT_LISTEN");
    if bind.is_empty() {
        log::info!("Audit receiver disabled; configure AUDIT_LISTEN and AUDIT_TOKEN to enable it");
        return std::future::pending().await;
    }
    let token = get_arg("AUDIT_TOKEN");
    let addr = audit_address(&bind, &token)?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    log::info!("Audit receiver listening on {}", listener.local_addr()?);
    serve(listener, AuditState { db, token }).await
}

fn audit_address(bind: &str, token: &str) -> ResultType<SocketAddr> {
    ensure!(token.len() >= 32 && token.is_ascii() && !token.bytes().any(|b| b.is_ascii_whitespace()),
        "AUDIT_TOKEN must contain at least 32 non-whitespace ASCII bytes");
    Ok(bind.parse()?)
}

async fn serve(listener: tokio::net::TcpListener, state: AuditState) -> ResultType<()> {
    let app = Router::new()
        .route("/api/audit/{kind}", post(receive))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(state);
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}

fn valid_record(value: &Value) -> bool {
    let device = value.get("id").and_then(Value::as_str).unwrap_or("");
    let nonce = value.get("nonce").and_then(Value::as_str).unwrap_or("");
    !device.is_empty() && device.len() <= 253 && uuid::Uuid::parse_str(nonce).is_ok()
}

async fn receive(
    State(state): State<AuditState>,
    ConnectInfo(source): ConnectInfo<SocketAddr>,
    Path(kind): Path<String>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> StatusCode {
    let supplied = headers.get("Authorization").and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
    if supplied.len() != state.token.len()
        || !sodiumoxide::utils::memcmp(supplied.as_bytes(), state.token.as_bytes()) {
        return StatusCode::UNAUTHORIZED;
    }
    if !matches!(kind.as_str(), "conn" | "file" | "alarm") {
        return StatusCode::NOT_FOUND;
    }
    if !valid_record(&value) {
        return StatusCode::BAD_REQUEST;
    }
    let device = value["id"].as_str().unwrap_or_default();
    let nonce = value["nonce"].as_str().unwrap_or_default();
    match state.db.store_audit(device, nonce, &kind, &source.ip().to_string(), &value.to_string()).await {
        Ok(AuditWrite::Conflict) => StatusCode::CONFLICT,
        Ok(_) => StatusCode::OK,
        Err(error) => {
            // Do not log submitted payloads or the bearer credential.
            log::error!("Audit record was not stored: {}", error);
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sqlx::Connection;

    #[test]
    fn audit_listener_rejects_unusable_credentials_and_addresses() {
        let token = "t".repeat(32);
        for invalid in [String::new(), "t".repeat(31), " ".repeat(32), format!("{}\n", token)] {
            assert!(audit_address("127.0.0.1:21114", &invalid).is_err());
        }
        assert!(audit_address("not-an-address", &token).is_err());
        assert_eq!(audit_address("127.0.0.1:21114", &token).unwrap().port(), 21114);
    }

    #[tokio::test]
    async fn audit_http_requires_credentials_and_persists_idempotently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.sqlite3");
        let path = path.to_str().unwrap();
        let db = Database::new(path).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/api/audit/conn", listener.local_addr().unwrap());
        let token = "audit-http-test-credential-at-least-32-bytes";
        let task = tokio::spawn(serve(listener, AuditState { db: db.clone(), token: token.into() }));
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let value = json!({"id":"device", "nonce":uuid::Uuid::new_v4().to_string(), "action":"open"});
        let mut reader = sqlx::SqliteConnection::connect(path).await.unwrap();
        for credential in ["", "incorrect"] {
            assert_eq!(client.post(&url).bearer_auth(credential).json(&value).send().await.unwrap().status().as_u16(), 401);
        }
        let rejected_count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_event").fetch_one(&mut reader).await.unwrap();
        assert_eq!(rejected_count, 0);
        for bad in [json!({"id":"device","nonce":"invalid"}), json!({"nonce":uuid::Uuid::new_v4().to_string()})] {
            assert_eq!(client.post(&url).bearer_auth(token).json(&bad).send().await.unwrap().status().as_u16(), 400);
        }
        for _ in 0..2 {
            let response = client.post(&url).bearer_auth(token).json(&value).send().await.unwrap();
            assert_eq!(response.status().as_u16(), 200);
            assert!(response.text().await.unwrap().is_empty());
        }
        // A duplicate must not overwrite the first event, including across retry sources.
        let mut changed = value.clone();
        changed["action"] = json!("modified");
        assert_eq!(client.post(&url).bearer_auth(token).json(&changed).send().await.unwrap().status().as_u16(), 409);
        let stored: (i64, String) = sqlx::query_as("SELECT count(*),payload FROM audit_event")
            .fetch_one(&mut reader).await.unwrap();
        assert_eq!(stored.0, 1);
        assert_eq!(serde_json::from_str::<Value>(&stored.1).unwrap(), value);
        let oversized = json!({"id":"device","nonce":uuid::Uuid::new_v4().to_string(),"data":"x".repeat(1024*1024)});
        assert_eq!(client.post(&url).bearer_auth(token).json(&oversized).send().await.unwrap().status().as_u16(), 413);
        assert_eq!(client.post(url.replace("/conn","/unknown")).bearer_auth(token).json(&value).send().await.unwrap().status().as_u16(), 404);
        task.abort();
    }
}
