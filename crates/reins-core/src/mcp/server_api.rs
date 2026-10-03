//! The two Reins server endpoints universal MCP needs for large results: `PUT /reins/api/blobs/output` (a file
//! made on the phone, answered with a download link) and `POST /reins/api/mcp/call` (the server makes one MCP call
//! for a heavy tool). Authenticated like the rest of the phone API, with one retry after a 401.

use reins_proto::blob::{BlobDownload, DEFAULT_BLOB_TTL_SECS};
use reins_proto::remote_mcp::{ProxyCall, ProxyCallResult};
use reqwest::{Method, RequestBuilder};
use serde::de::DeserializeOwned;

use crate::CoreError;
use crate::http::error_text;
use crate::phone_api::ApiFailure;
use crate::session::Session;

async fn authorized<T: DeserializeOwned>(
    session: &Session,
    build: impl Fn(&str) -> RequestBuilder,
) -> Result<T, CoreError> {
    let mut retried = false;
    loop {
        let token = session.access_token().await?;
        let resp = build(&token).send().await?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            if retried {
                return Err(CoreError::NotLoggedIn);
            }
            retried = true;
            session.invalidate().await;
            continue;
        }
        let body = resp.text().await?;
        if !status.is_success() {
            let (code, message) = error_text(status, &body);
            return Err(ApiFailure::Status {
                status: status.as_u16(),
                code,
                message,
            }
            .into_core());
        }
        return serde_json::from_str(&body).map_err(|_| CoreError::Network {
            reason: "invalid response from the server".to_owned(),
        });
    }
}

fn request(session: &Session, method: Method, path: &str, token: &str) -> RequestBuilder {
    crate::phone_api::with_device_key(
        session.http.request(method, session.server.join(&format!("/reins/api{path}"))).bearer_auth(token),
        session.device_key(),
    )
}

/// Hands the server a result made on the phone; the AI gets the download link.
pub async fn put_output(
    session: &Session,
    connection_id: &str,
    name: &str,
    content_type: &str,
    bytes: &[u8],
) -> Result<BlobDownload, CoreError> {
    let ttl = DEFAULT_BLOB_TTL_SECS.to_string();
    authorized(session, |token| {
        request(session, Method::PUT, "/blobs/output", token)
            .query(&[("connection_id", connection_id), ("name", name), ("ttl_secs", ttl.as_str())])
            .header("Content-Type", content_type)
            .body(bytes.to_vec())
    })
    .await
}

/// Has the server make one MCP call for the phone.
pub async fn proxy_call(session: &Session, call: &ProxyCall) -> Result<ProxyCallResult, CoreError> {
    authorized(session, |token| request(session, Method::POST, "/mcp/call", token).json(call)).await
}
