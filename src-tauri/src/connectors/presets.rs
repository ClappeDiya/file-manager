//! Provider presets: the one catalog the connection form and the backup
//! engine use to fill in endpoints for S3-compatible stores and hosted
//! SFTP services. Templates use `{region}`, `{account}`, `{domain}`,
//! `{subdomain}` and `{user}`.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderPreset {
    pub id: &'static str,
    pub label: &'static str,
    /// Connection protocol (`s3`, `sftp`…).
    pub protocol: &'static str,
    /// Host / endpoint template.
    pub host: &'static str,
    pub port: Option<u16>,
    pub default_region: Option<&'static str>,
    /// Path-style addressing (S3).
    pub path_style: bool,
    /// Values the user must enter for the templates.
    pub fields: &'static [&'static str],
    /// How the provider expects the username (`{user}$ {domain}`…).
    pub username: Option<&'static str>,
    pub help: &'static str,
}

const fn s3(id: &'static str, label: &'static str, host: &'static str, region: Option<&'static str>, fields: &'static [&'static str], help: &'static str) -> ProviderPreset {
    ProviderPreset { id, label, protocol: "s3", host, port: None, default_region: region, path_style: false, fields, username: None, help }
}

pub const PRESETS: &[ProviderPreset] = &[
    s3("aws", "Amazon S3", "s3.{region}.amazonaws.com", Some("us-east-1"), &["region"], ""),
    s3("r2", "Cloudflare R2", "https://{account}.r2.cloudflarestorage.com", Some("auto"), &["account"], "Account ID from the R2 dashboard; create an R2 API token for the access key and secret."),
    s3("linode", "Linode / Akamai Object Storage", "https://{region}.linodeobjects.com", Some("us-east-1"), &["region"], "Region such as us-east-1, eu-central-1 or ap-south-1."),
    s3("filebase", "Filebase", "https://s3.filebase.com", Some("us-east-1"), &[], "Access keys from the Filebase console."),
    s3("wasabi", "Wasabi", "https://s3.{region}.wasabisys.com", Some("us-east-1"), &["region"], ""),
    s3("backblaze", "Backblaze B2 (S3 API)", "https://s3.{region}.backblazeb2.com", Some("us-west-004"), &["region"], "Region from the bucket's endpoint, e.g. us-west-004."),
    s3("digitalocean", "DigitalOcean Spaces", "https://{region}.digitaloceanspaces.com", Some("nyc3"), &["region"], ""),
    s3("scaleway", "Scaleway Object Storage", "https://s3.{region}.scw.cloud", Some("fr-par"), &["region"], ""),
    s3("gcs", "Google Cloud Storage (S3 API)", "https://storage.googleapis.com", Some("auto"), &[], "Create HMAC keys in Cloud Storage > Settings > Interoperability."),
    s3("storj", "Storj", "https://gateway.storjshare.io", Some("us-east-1"), &[], ""),
    s3("idrive_e2", "IDrive e2", "https://{region}", None, &["region"], "Enter the full endpoint host shown in IDrive e2 as the region."),
    s3("aliyun_oss", "Alibaba Cloud OSS", "https://oss-{region}.aliyuncs.com", Some("cn-hangzhou"), &["region"], ""),
    s3("tencent_cos", "Tencent Cloud COS", "https://cos.{region}.myqcloud.com", Some("ap-guangzhou"), &["region"], ""),
    ProviderPreset { id: "minio", label: "MinIO (self-hosted)", protocol: "s3", host: "http://localhost:9000", port: None, default_region: Some("us-east-1"), path_style: true, fields: &[], username: None, help: "Replace with your MinIO URL." },
    ProviderPreset {
        id: "egnyte",
        label: "Egnyte",
        protocol: "sftp",
        host: "ftp-{domain}.egnyte.com",
        port: Some(22),
        default_region: None,
        path_style: false,
        fields: &["domain"],
        username: Some("{user}${domain}"),
        help: "Domain is the first part of your Egnyte URL (acme for acme.egnyte.com). SFTP must be enabled for your account.",
    },
    ProviderPreset {
        id: "sharefile",
        label: "Citrix ShareFile",
        protocol: "sftp",
        host: "{subdomain}.sharefileftp.com",
        port: Some(22),
        default_region: None,
        path_style: false,
        fields: &["subdomain"],
        username: Some("{subdomain}/{user}"),
        help: "Subdomain is the first part of your ShareFile URL (acme for acme.sharefile.com).",
    },
];

pub fn find(id: &str) -> Option<&'static ProviderPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// Fill `{name}` placeholders from `values` (unknown ones stay as-is).
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    values.iter().fold(template.to_string(), |s, (k, v)| s.replace(&format!("{{{k}}}"), v))
}

/// The S3 endpoint URL for a preset and region (backup destinations).
pub fn s3_endpoint(id: &str, region: &str) -> Option<String> {
    let p = find(id).filter(|p| p.protocol == "s3" && !p.fields.contains(&"account"))?;
    let host = fill(p.host, &[("region", region)]);
    Some(if host.contains("://") { host } else { format!("https://{host}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_consistent() {
        let mut ids: Vec<_> = PRESETS.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len(), "ids are unique");
        for p in PRESETS {
            // Every placeholder in a template is a declared field (or {user}).
            for t in [Some(p.host), p.username].into_iter().flatten() {
                for name in t.split('{').skip(1).filter_map(|x| x.split_once('}').map(|(n, _)| n)) {
                    assert!(name == "user" || p.fields.contains(&name), "{}: {{{name}}} not in fields", p.id);
                }
            }
        }
    }

    #[test]
    fn fills_templates() {
        let r2 = find("r2").unwrap();
        assert_eq!(fill(r2.host, &[("account", "abc123")]), "https://abc123.r2.cloudflarestorage.com");
        let eg = find("egnyte").unwrap();
        assert_eq!(fill(eg.username.unwrap(), &[("user", "ann"), ("domain", "acme")]), "ann$acme");
        assert_eq!(fill(eg.host, &[("domain", "acme")]), "ftp-acme.egnyte.com");
        assert_eq!(s3_endpoint("aws", "eu-west-1").unwrap(), "https://s3.eu-west-1.amazonaws.com");
        assert_eq!(s3_endpoint("linode", "us-east-1").unwrap(), "https://us-east-1.linodeobjects.com");
        assert_eq!(s3_endpoint("aliyun_oss", "cn-hangzhou").unwrap(), "https://oss-cn-hangzhou.aliyuncs.com");
        assert_eq!(s3_endpoint("storj", "x").unwrap(), "https://gateway.storjshare.io");
        assert!(s3_endpoint("r2", "auto").is_none(), "needs the account id");
        assert!(s3_endpoint("egnyte", "x").is_none() && s3_endpoint("nope", "x").is_none());
    }
}
