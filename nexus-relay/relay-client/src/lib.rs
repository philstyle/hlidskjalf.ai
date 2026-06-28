pub mod error;
pub mod types;

use chrono::{DateTime, Utc};
use error::ClientError;
use types::{
    AppendRequest, AppendResponse, BlobUploadResponse, HeadResponse, MeResponse, ReadResponse,
};
use uuid::Uuid;

pub struct RelayClient {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl RelayClient {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub async fn append(
        &self,
        ledger_id: Uuid,
        msg_type: &str,
        payload: serde_json::Value,
        correlation_id: Option<Uuid>,
        sent_at: Option<DateTime<Utc>>,
        attachments: Option<serde_json::Value>,
    ) -> Result<AppendResponse, ClientError> {
        let url = format!("{}/ledger/{}/append", self.base_url, ledger_id);
        let body = AppendRequest {
            msg_type: msg_type.to_string(),
            correlation_id,
            sent_at,
            payload,
            attachments,
        };
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        self.parse_response(resp).await
    }

    pub async fn read(
        &self,
        ledger_id: Uuid,
        since: Option<i64>,
        limit: Option<i64>,
    ) -> Result<ReadResponse, ClientError> {
        let mut url = format!("{}/ledger/{}/read", self.base_url, ledger_id);
        let mut params: Vec<String> = Vec::new();
        if let Some(s) = since {
            params.push(format!("since={}", s));
        }
        if let Some(l) = limit {
            params.push(format!("limit={}", l));
        }
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;

        self.parse_response(resp).await
    }

    pub async fn head(&self, ledger_id: Uuid) -> Result<HeadResponse, ClientError> {
        let url = format!("{}/ledger/{}/head", self.base_url, ledger_id);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;

        self.parse_response(resp).await
    }

    pub async fn upload_blob(
        &self,
        content: Vec<u8>,
        filename: &str,
    ) -> Result<BlobUploadResponse, ClientError> {
        let url = format!("{}/blobs", self.base_url);
        let form = reqwest::multipart::Form::new()
            .part(
                "file",
                reqwest::multipart::Part::bytes(content).file_name(filename.to_string()),
            )
            .text("filename", filename.to_string());
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await?;
        self.parse_response(resp).await
    }

    pub async fn download_blob(&self, sha: &str) -> Result<Vec<u8>, ClientError> {
        let url = format!("{}/blobs/{}", self.base_url, sha);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;
        if resp.status().is_success() {
            Ok(resp.bytes().await?.to_vec())
        } else {
            let status_code = resp.status().as_u16();
            let body: serde_json::Value = resp
                .json()
                .await
                .unwrap_or_else(|_| serde_json::json!({"error": "unknown error"}));
            let message = body["error"]
                .as_str()
                .unwrap_or("unknown error")
                .to_string();
            Err(ClientError::Api {
                status: status_code,
                message,
            })
        }
    }

    pub async fn get_me(&self) -> Result<MeResponse, ClientError> {
        let url = format!("{}/participants/me", self.base_url);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.api_key)
            .send()
            .await?;

        self.parse_response(resp).await
    }

    async fn parse_response<T: serde::de::DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T, ClientError> {
        let status = resp.status();
        if status.is_success() {
            resp.json::<T>()
                .await
                .map_err(|e| ClientError::Deserialize(e.to_string()))
        } else {
            let status_code = status.as_u16();
            let body: serde_json::Value = resp
                .json()
                .await
                .unwrap_or_else(|_| serde_json::json!({"error": "unknown error"}));
            let message = body["error"]
                .as_str()
                .unwrap_or("unknown error")
                .to_string();
            Err(ClientError::Api {
                status: status_code,
                message,
            })
        }
    }
}
