//! Native object-store features against in-process mocks that verify the
//! authentication of every request: the AWS credential chain and AssumeRole,
//! storage classes per volume kind, point-in-time reads of versioned
//! buckets, sharded layouts with parallel prefix listings, Azure Entra ID
//! (service principal, certificate, federated, managed identity) and GCS
//! read-only scopes.

use super::mock_s3;
use super::tests::{backup_roundtrip_with, exercise};
use super::*;
use crate::connectors::aws_chain::{AssumeRoleOptions, ChainEnv, CredentialSource};
use std::collections::HashMap;
use std::sync::Arc;
use ufop_core::backup::layout::RepoLayout;

fn s3cfg(base: &str) -> s3::S3Config {
    s3::S3Config { preset: "custom".into(), endpoint: base.into(), bucket: "bkt".into(), prefix: "pc".into(), access_key_id: "AKIDBASE".into(), path_style: true, part_size_mb: 0, ..Default::default() }
}

fn env(pairs: &[(&str, String)]) -> ChainEnv {
    ChainEnv::fixed(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<HashMap<_, _>>())
}

fn store(cfg: s3::S3Config, secret: Option<&str>, e: ChainEnv) -> s3::S3Store {
    s3::S3Store::with_env(cfg, secret.map(String::from), e).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn s3_mock_static_keys_sharded_backup_and_bad_signature() {
    let (base, st) = mock_s3::serve().await;
    let s = store(s3cfg(&base), Some("basesecret"), ChainEnv::fixed(HashMap::new()));
    exercise(&s, 300_000).await;
    // A wrong secret fails the mock's signature check.
    let bad = store(s3cfg(&base), Some("wrong"), ChainEnv::fixed(HashMap::new()));
    assert!(bad.list().await.unwrap_err().user_message().contains("SignatureDoesNotMatch"));
    // Sharded layout end to end: volumes go under shard prefixes, listed in parallel.
    let b = Arc::new(StoreBackend::new(Box::new(store(s3cfg(&base), Some("basesecret"), ChainEnv::fixed(HashMap::new()))), None).with_list_parallelism(4));
    assert!(ufop_core::backup::Backend::supports_subfolders(b.as_ref()));
    let b2 = b.clone();
    tokio::task::spawn_blocking(move || backup_roundtrip_with(b2.as_ref(), RepoLayout::Sharded { prefix_len: 1 })).await.unwrap();
    // Parallel listing = one delimited top listing + one per shard folder, same result as serial.
    let serial = store(s3cfg(&base), Some("basesecret"), ChainEnv::fixed(HashMap::new()));
    let mut a = serial.list_recursive(1).await.unwrap();
    let mut p = serial.list_recursive(4).await.unwrap();
    a.sort_by(|x, y| x.name.cmp(&y.name));
    p.sort_by(|x, y| x.name.cmp(&y.name));
    assert_eq!(a, p);
    let folders: std::collections::BTreeSet<_> = p.iter().filter_map(|f| f.name.split_once('/').map(|(d, _)| d.to_string())).collect();
    assert!(!folders.is_empty());
    let before = st.lock().unwrap().log.len();
    serial.list_recursive(4).await.unwrap();
    assert_eq!(st.lock().unwrap().log.len() - before, 1 + folders.len());
}

#[tokio::test(flavor = "multi_thread")]
async fn s3_credential_chain_sources() {
    let (base, st) = mock_s3::serve().await;
    let d = tempfile::tempdir().unwrap();
    // Environment.
    let e = env(&[("AWS_ACCESS_KEY_ID", "AKIDBASE".into()), ("AWS_SECRET_ACCESS_KEY", "basesecret".into())]);
    let s = store(s3::S3Config { credentials: CredentialSource::Environment, access_key_id: String::new(), ..s3cfg(&base) }, None, e);
    s.create_folder().await.unwrap();
    s.put("ufop-a.dlist", b"env".to_vec()).await.unwrap();
    // Web identity: token file exchanged at the (overridden) STS endpoint.
    let tf = d.path().join("token");
    std::fs::write(&tf, "oidc-token\n").unwrap();
    let wi = CredentialSource::WebIdentity { role_arn: "arn:aws:iam::1:role/Backup".into(), token_file: tf.to_string_lossy().into() };
    let s = store(s3::S3Config { credentials: wi, ..s3cfg(&base) }, None, env(&[("AWS_ENDPOINT_URL_STS", base.clone())]));
    assert_eq!(s.get("ufop-a.dlist").await.unwrap(), b"env");
    assert!(st.lock().unwrap().sts.last().unwrap().contains("AssumeRoleWithWebIdentity"));
    std::fs::write(&tf, "wrong").unwrap();
    let s = store(s3::S3Config { credentials: CredentialSource::WebIdentity { role_arn: "arn:aws:iam::1:role/B".into(), token_file: tf.to_string_lossy().into() }, ..s3cfg(&base) }, None, env(&[("AWS_ENDPOINT_URL_STS", base.clone())]));
    assert!(s.list().await.unwrap_err().user_message().contains("InvalidIdentityToken"));
    // Instance metadata (IMDSv2 session token, role name, credentials).
    let s = store(s3::S3Config { credentials: CredentialSource::InstanceMetadata, ..s3cfg(&base) }, None, env(&[("AWS_EC2_METADATA_SERVICE_ENDPOINT", base.clone())]));
    assert_eq!(s.list().await.unwrap().len(), 1);
    let log = st.lock().unwrap().log.clone();
    assert!(log.iter().any(|(m, p, _)| m == "PUT" && p == "/latest/api/token"));
    // Auto: no variables or profile → container credentials (with the auth header).
    let home = d.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let e = env(&[("HOME", home.to_string_lossy().into()), ("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "/ecs/creds".into()), ("UFOP_AWS_ECS_ENDPOINT", base.clone()), ("AWS_CONTAINER_AUTHORIZATION_TOKEN", "ecs-secret".into())]);
    let s = store(s3::S3Config { credentials: CredentialSource::Auto, ..s3cfg(&base) }, None, e);
    assert_eq!(s.list().await.unwrap().len(), 1);
    assert!(st.lock().unwrap().log.iter().any(|(_, p, _)| p == "/ecs/creds"));
    // Auto with nothing at all reports every place it looked.
    let e = env(&[("HOME", home.to_string_lossy().into()), ("AWS_EC2_METADATA_SERVICE_ENDPOINT", "http://127.0.0.1:9".into())]);
    let msg = store(s3::S3Config { credentials: CredentialSource::Auto, ..s3cfg(&base) }, None, e).list().await.unwrap_err().user_message();
    assert!(msg.contains("No AWS credentials found"), "{msg}");
    // Profile: role_arn + source_profile → AssumeRole signed with the source keys.
    let cf = d.path().join("credentials");
    std::fs::write(&cf, "[base]\naws_access_key_id = AKIDBASE\naws_secret_access_key = basesecret\n").unwrap();
    let cfgf = d.path().join("config");
    std::fs::write(&cfgf, "[profile ops]\nrole_arn = arn:aws:iam::1:role/Ops\nsource_profile = base\n").unwrap();
    let e = env(&[("AWS_CONFIG_FILE", cfgf.to_string_lossy().into()), ("AWS_ENDPOINT_URL_STS", base.clone())]);
    let s = store(s3::S3Config { credentials: CredentialSource::Profile { name: "ops".into(), file: Some(cf.to_string_lossy().into()) }, ..s3cfg(&base) }, None, e);
    assert_eq!(s.list().await.unwrap().len(), 1);
    assert!(st.lock().unwrap().sts.last().unwrap().contains("Action=AssumeRole&"));
}

#[tokio::test(flavor = "multi_thread")]
async fn s3_assume_role_is_cached_and_refreshed_before_expiry() {
    let (base, st) = mock_s3::serve().await;
    let role = AssumeRoleOptions {
        role: crate::core::types::AssumeRoleConfig { role_arn: "arn:aws:iam::123456789012:role/Writer".into(), external_id: Some("ext-7".into()), session_name: Some("nightly".into()), duration_secs: Some(900) },
        sts_endpoint: Some(base.clone()),
        region: Some("eu-west-1".into()),
    };
    let s = store(s3::S3Config { assume_role: Some(role), ..s3cfg(&base) }, Some("basesecret"), ChainEnv::fixed(HashMap::new()));
    s.create_folder().await.unwrap();
    s.put("ufop-x.dlist", b"1".to_vec()).await.unwrap();
    s.list().await.unwrap();
    {
        let g = st.lock().unwrap();
        assert_eq!(g.issued, 1, "one AssumeRole for many requests");
        let body = &g.sts[0];
        assert!(body.contains("ExternalId=ext-7") && body.contains("RoleSessionName=nightly") && body.contains("DurationSeconds=900"), "{body}");
    }
    // The base keys themselves never sign S3 requests: revoke them and it still works.
    st.lock().unwrap().keys.remove("AKIDBASE");
    s.list().await.unwrap();
    // Credentials that expire inside the refresh window are fetched again.
    st.lock().unwrap().keys.insert("AKIDBASE".into(), ("basesecret".into(), None));
    st.lock().unwrap().ttl_secs = 60;
    let s = store(s3::S3Config { assume_role: Some(AssumeRoleOptions { role: crate::core::types::AssumeRoleConfig { role_arn: "arn:aws:iam::1:role/W".into(), ..Default::default() }, sts_endpoint: Some(base.clone()), region: None }), ..s3cfg(&base) }, Some("basesecret"), ChainEnv::fixed(HashMap::new()));
    let before = st.lock().unwrap().issued;
    s.list().await.unwrap();
    s.list().await.unwrap();
    assert_eq!(st.lock().unwrap().issued - before, 2, "short-lived credentials are refreshed");
}

#[tokio::test(flavor = "multi_thread")]
async fn s3_storage_class_per_volume_kind() {
    let (base, st) = mock_s3::serve().await;
    let by = ClassByKind { dblock: "STANDARD_IA".into(), dindex: String::new(), dlist: "STANDARD".into(), other: "ONEZONE_IA".into() };
    let s = store(s3::S3Config { storage_class: "INTELLIGENT_TIERING".into(), storage_class_by_kind: by, ..s3cfg(&base) }, Some("basesecret"), ChainEnv::fixed(HashMap::new()));
    s.create_folder().await.unwrap();
    for n in ["ab/ufop-b1.dblock.aes", "ufop-i1.dindex", "ufop-20240101T000000Z.dlist.gpg", "ufop-repo.json"] {
        s.put(n, b"x".to_vec()).await.unwrap();
    }
    let g = st.lock().unwrap();
    let class = |k: &str| g.objects[&format!("bkt/pc/{k}")][0].class.clone();
    assert_eq!(class("ab/ufop-b1.dblock.aes").as_deref(), Some("STANDARD_IA"));
    assert_eq!(class("ufop-i1.dindex").as_deref(), Some("INTELLIGENT_TIERING"), "empty entry: the single class");
    assert_eq!(class("ufop-20240101T000000Z.dlist.gpg").as_deref(), Some("STANDARD"));
    assert_eq!(class("ufop-repo.json").as_deref(), Some("ONEZONE_IA"));
    drop(g);
    // Archive tiers are refused for any kind, with advice.
    for k in ["dblock", "dindex", "dlist"] {
        let mut by = ClassByKind::default();
        match k {
            "dblock" => by.dblock = "DEEP_ARCHIVE".into(),
            "dindex" => by.dindex = "glacier".into(),
            _ => by.dlist = "GLACIER".into(),
        }
        let e = s3::S3Store::with_env(s3::S3Config { storage_class_by_kind: by, ..s3cfg(&base) }, Some("basesecret".into()), ChainEnv::fixed(HashMap::new())).err().unwrap();
        assert!(e.user_message().contains(k) && e.user_message().contains("restore before every read"), "{}", e.user_message());
    }
    // Instant-retrieval Glacier is fine.
    assert!(s3::S3Store::with_env(s3::S3Config { storage_class_by_kind: ClassByKind { dblock: "GLACIER_IR".into(), ..Default::default() }, ..s3cfg(&base) }, Some("basesecret".into()), ChainEnv::fixed(HashMap::new())).is_ok());
}

#[tokio::test(flavor = "multi_thread")]
async fn s3_point_in_time_reads_versions_and_refuses_writes() {
    let (base, st) = mock_s3::serve().await;
    let t = |h: u32| chrono::DateTime::parse_from_rfc3339(&format!("2026-01-01T{h:02}:00:00Z")).unwrap().with_timezone(&chrono::Utc);
    let live = store(s3cfg(&base), Some("basesecret"), ChainEnv::fixed(HashMap::new()));
    live.create_folder().await.unwrap();
    st.lock().unwrap().now = Some(t(1));
    live.put("ufop-a.dlist", b"a1".to_vec()).await.unwrap();
    live.put("0f/ufop-b.dblock", b"b1".to_vec()).await.unwrap();
    st.lock().unwrap().now = Some(t(2));
    live.put("ufop-a.dlist", b"a2-ransomware".to_vec()).await.unwrap();
    live.delete("0f/ufop-b.dblock").await.unwrap();
    live.put("ufop-c.dlist", b"c2".to_vec()).await.unwrap();
    st.lock().unwrap().now = Some(t(3));
    live.delete("ufop-a.dlist").await.unwrap();
    st.lock().unwrap().page = 2; // several ListObjectVersions pages

    let at = |h: u32| store(s3::S3Config { point_in_time: Some(t(h)), ..s3cfg(&base) }, Some("basesecret"), ChainEnv::fixed(HashMap::new()));
    // Before the attack: the old dlist, the block in its shard, no c.
    let p = at(1);
    assert!(p.is_read_only() && p.describe().contains("@ 2026-01-01T01:00:00Z"));
    let mut names: Vec<_> = p.list_recursive(1).await.unwrap().into_iter().map(|f| (f.name, f.size)).collect();
    names.sort();
    assert_eq!(names, vec![("0f/ufop-b.dblock".to_string(), 2), ("ufop-a.dlist".to_string(), 2)]);
    assert_eq!(p.list().await.unwrap().len(), 1, "flat listing leaves shard folders out");
    assert_eq!(p.get("ufop-a.dlist").await.unwrap(), b"a1");
    assert_eq!(p.get("0f/ufop-b.dblock").await.unwrap(), b"b1");
    assert!(http::is_not_found(&p.get("ufop-c.dlist").await.unwrap_err()));
    assert_eq!(p.modified("ufop-a.dlist").await.unwrap(), Some(t(1)));
    // Reads used versionId.
    assert!(st.lock().unwrap().log.iter().any(|(m, _, q)| m == "GET" && q.contains("versionId=")));
    // Later: the delete marker hides b, a is the overwritten copy; at 3 a is gone.
    let p2 = at(2);
    let n2: Vec<_> = p2.list_recursive(1).await.unwrap().into_iter().map(|f| f.name).collect();
    assert!(n2.contains(&"ufop-c.dlist".to_string()) && !n2.iter().any(|n| n.contains("ufop-b")), "{n2:?}");
    assert_eq!(p2.get("ufop-a.dlist").await.unwrap(), b"a2-ransomware");
    assert!(!at(3).list().await.unwrap().iter().any(|f| f.name == "ufop-a.dlist"));
    // Every write is refused before reaching the bucket.
    let writes = st.lock().unwrap().log.iter().filter(|(m, _, _)| m == "PUT" || m == "DELETE").count();
    for e in [p.put("x", vec![]).await.unwrap_err(), p.delete("ufop-a.dlist").await.unwrap_err(), p.lock("ufop-a.dlist", chrono::Utc::now(), ufop_core::backup::backend::LockMode::Governance).await.unwrap_err()] {
        assert!(matches!(e, crate::core::error::AppError::PermissionDenied { .. }), "{e:?}");
    }
    assert_eq!(st.lock().unwrap().log.iter().filter(|(m, _, _)| m == "PUT" || m == "DELETE").count(), writes);
    assert!(!p.supports_lock());
    let b = StoreBackend::new(Box::new(at(1)), None);
    let r = tokio::task::spawn_blocking(move || ufop_core::backup::Backend::is_read_only(&b)).await.unwrap();
    assert!(r);
}
