//! Backblaze B2 native API (including B2 Object Lock file retention).

use super::http;
use super::{direct_child, join_key, BoxFut, ObjectStore};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use ufop_core::backup::backend::LockMode;
use ufop_core::backup::RemoteFile;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct B2Config {
    pub key_id: String,
    pub bucket: String,
    pub prefix: String,
    /// API base (tests); empty = `https://api.backblazeb2.com`.
    pub api_url: String,
}


#[derive(Clone)]
struct Session {
    api: String,
    download: String,
    token: String,
    bucket_id: String,
}

pub struct B2Store {
    cfg: B2Config,
    key: String,
    http: reqwest::Client,
    session: Mutex<Session>,
}

impl B2Store {
    pub async fn connect(cfg: B2Config, key: String) -> Result<Self, AppError> {
        if cfg.key_id.is_empty() || cfg.bucket.is_empty() {
            return Err(AppError::validation("Enter the B2 key id and bucket"));
        }
        let http = http::client(&Default::default())?;
        let s = Self::authorize(&http, &cfg, &key).await?;
        Ok(Self { cfg, key, http, session: Mutex::new(s) })
    }

    async fn authorize(http: &reqwest::Client, cfg: &B2Config, key: &str) -> Result<Session, AppError> {
        let base = if cfg.api_url.is_empty() { "https://api.backblazeb2.com" } else { cfg.api_url.trim_end_matches('/') };
        let r = http.get(format!("{base}/b2api/v2/b2_authorize_account")).basic_auth(&cfg.key_id, Some(key)).send().await.map_err(|e| http::net("B2 sign-in", e))?;
        let v: serde_json::Value = http::check(r, "B2 sign-in").await?.json().await.map_err(|e| http::net("B2 sign-in", e))?;
        let api = v["apiUrl"].as_str().unwrap_or_default().to_string();
        let token = v["authorizationToken"].as_str().unwrap_or_default().to_string();
        let account = v["accountId"].as_str().unwrap_or_default().to_string();
        let r = http
            .post(format!("{api}/b2api/v2/b2_list_buckets"))
            .header("authorization", &token)
            .json(&serde_json::json!({ "accountId": account, "bucketName": cfg.bucket }))
            .send()
            .await
            .map_err(|e| http::net("B2 bucket lookup", e))?;
        let b: serde_json::Value = http::check(r, "B2 bucket lookup").await?.json().await.map_err(|e| http::net("B2 bucket lookup", e))?;
        let bucket_id = b["buckets"].as_array().and_then(|a| a.iter().find(|x| x["bucketName"] == cfg.bucket.as_str())).and_then(|x| x["bucketId"].as_str()).map(String::from);
        let bucket_id = bucket_id.ok_or_else(|| AppError::not_found(format!("B2 bucket '{}' not found or not allowed for this key", cfg.bucket)))?;
        Ok(Session { api, download: v["downloadUrl"].as_str().unwrap_or_default().to_string(), token, bucket_id })
    }

    /// POST a B2 API call, signing in again once when the token expired.
    async fn call(&self, op: &str, body: serde_json::Value) -> Result<serde_json::Value, AppError> {
        for attempt in 0..2 {
            let s = self.session.lock().await.clone();
            let r = self.http.post(format!("{}/b2api/v2/{op}", s.api)).header("authorization", &s.token).json(&body).send().await.map_err(|e| http::net(op, e))?;
            if r.status().as_u16() == 401 && attempt == 0 {
                *self.session.lock().await = Self::authorize(&self.http, &self.cfg, &self.key).await?;
                continue;
            }
            return http::check(r, op).await?.json().await.map_err(|e| http::net(op, e));
        }
        unreachable!()
    }

    async fn versions(&self, key: &str) -> Result<Vec<String>, AppError> {
        let s = self.session.lock().await.clone();
        let v = self.call("b2_list_file_versions", serde_json::json!({ "bucketId": s.bucket_id, "startFileName": key, "prefix": key, "maxFileCount": 100 })).await?;
        Ok(v["files"].as_array().into_iter().flatten().filter(|f| f["fileName"] == key).filter_map(|f| f["fileId"].as_str().map(String::from)).collect())
    }
}

impl ObjectStore for B2Store {
    fn describe(&self) -> String {
        format!("b2://{}/{}", self.cfg.bucket, self.cfg.prefix.trim_matches('/'))
    }
    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            let prefix = self.cfg.prefix.trim_matches('/');
            let pfx = if prefix.is_empty() { String::new() } else { format!("{prefix}/") };
            let bucket_id = self.session.lock().await.bucket_id.clone();
            let mut out = Vec::new();
            let mut start: Option<String> = None;
            loop {
                let mut body = serde_json::json!({ "bucketId": bucket_id, "prefix": pfx, "delimiter": "/", "maxFileCount": 1000 });
                if let Some(s) = &start {
                    body["startFileName"] = s.clone().into();
                }
                let v = self.call("b2_list_file_names", body).await?;
                for f in v["files"].as_array().into_iter().flatten() {
                    if f["action"] != "upload" {
                        continue;
                    }
                    if let Some(n) = direct_child(prefix, f["fileName"].as_str().unwrap_or_default()) {
                        out.push(RemoteFile { name: n.to_string(), size: f["contentLength"].as_u64().unwrap_or(0) });
                    }
                }
                start = v["nextFileName"].as_str().map(String::from);
                if start.is_none() {
                    break;
                }
            }
            Ok(out)
        })
    }
    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let bucket_id = self.session.lock().await.bucket_id.clone();
            let up = self.call("b2_get_upload_url", serde_json::json!({ "bucketId": bucket_id })).await?;
            let sha1 = { use sha1::Digest; hex::encode(sha1::Sha1::digest(&data)) };
            let r = self
                .http
                .post(up["uploadUrl"].as_str().unwrap_or_default())
                .header("authorization", up["authorizationToken"].as_str().unwrap_or_default())
                .header("X-Bz-File-Name", urlencoding::encode(&key).into_owned())
                .header("Content-Type", "b2/x-auto")
                .header("X-Bz-Content-Sha1", sha1)
                .body(data)
                .send()
                .await
                .map_err(|e| http::net("B2 upload", e))?;
            http::check(r, "B2 upload").await?;
            Ok(())
        })
    }
    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let s = self.session.lock().await.clone();
            let r = self.http.get(format!("{}/file/{}/{}", s.download, self.cfg.bucket, super::s3::uri_encode(&key, true))).header("authorization", &s.token).send().await.map_err(|e| http::net("B2 download", e))?;
            Ok(http::check(r, "B2 download").await?.bytes().await.map_err(|e| http::net("B2 download", e))?.to_vec())
        })
    }
    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            for id in self.versions(&key).await? {
                self.call("b2_delete_file_version", serde_json::json!({ "fileName": key, "fileId": id })).await?;
            }
            Ok(())
        })
    }
    fn supports_lock(&self) -> bool {
        true
    }
    fn lock<'a>(&'a self, name: &'a str, until: chrono::DateTime<chrono::Utc>, mode: LockMode) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let m = match mode {
                LockMode::Governance => "governance",
                LockMode::Compliance => "compliance",
            };
            for id in self.versions(&key).await? {
                self.call("b2_update_file_retention", serde_json::json!({ "fileName": key, "fileId": id, "fileRetention": { "mode": m, "retainUntilTimestamp": until.timestamp_millis() } })).await?;
            }
            Ok(())
        })
    }
}
