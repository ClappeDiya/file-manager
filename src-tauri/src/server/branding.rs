//! Web UI branding for `ufop-server`: a title prefix, product name, logo,
//! accent color and a little custom CSS.
//!
//! Three layers, highest first:
//! 1. the management console's managed settings (`serverBranding`; read-only
//!    here, the admin route refuses changes while it is set),
//! 2. what an administrator saved (`PUT /api/v1/server/branding`, kept in
//!    the server's settings database),
//! 3. the command line / config file (`--branding-file`, `--brand-*`).
//!
//! A field left empty in a layer falls through to the next one. The public
//! `GET /api/v1/branding` returns only presentation values; the logo and the
//! stylesheet are served at `/branding/logo` and `/branding/custom.css`,
//! and `index.html` is rewritten on the way out (title, accent variable,
//! stylesheet link) so the first paint is already branded. A custom web UI
//! folder is the existing `--webroot` flag (command line only).

use super::{ApiError, ApiResult, ServerState};
use crate::core::error::AppError;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use ufop_core::agent::managed::{check_accent_color, check_brand_text, check_custom_css, decode_logo_data_uri, ManagedServerBranding, MAX_BRAND_LOGO};

const SETTINGS_KEY: &str = "server_branding";

/// One layer of branding (empty / `None` = not set in this layer).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Branding {
    pub title_prefix: String,
    pub product_name: String,
    /// `data:image/…;base64,…`, or (command line / config file only) a file path.
    pub logo: String,
    pub accent_color: String,
    pub custom_css: String,
}

impl Branding {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Check every field; `allow_logo_path` only for the command line layer
    /// (an administrator over HTTP must not make the server publish files).
    pub fn validate(&self, allow_logo_path: bool) -> Result<(), AppError> {
        let v = |r: Result<(), String>| r.map_err(AppError::validation);
        v(check_brand_text("Title prefix", &self.title_prefix))?;
        v(check_brand_text("Product name", &self.product_name))?;
        if !self.accent_color.trim().is_empty() {
            v(check_accent_color(&self.accent_color))?;
        }
        v(check_custom_css(&self.custom_css))?;
        let logo = self.logo.trim();
        if logo.starts_with("data:") {
            v(decode_logo_data_uri(logo).map(|_| ()))?;
        } else if !logo.is_empty() {
            if !allow_logo_path {
                return Err(AppError::validation("The logo must be a data: URI (data:image/png;base64,…); file paths are only accepted on the server's command line"));
            }
            let p = PathBuf::from(logo);
            let len = std::fs::metadata(&p).map_err(|e| AppError::validation(format!("Logo file {}: {e}", p.display())))?.len();
            if len == 0 || len as usize > MAX_BRAND_LOGO {
                return Err(AppError::validation(format!("The logo file must be between 1 byte and {} KiB", MAX_BRAND_LOGO / 1024)));
            }
            logo_mime(&p).ok_or_else(|| AppError::validation("The logo file must be .png, .jpg, .svg, .webp or .gif"))?;
        }
        Ok(())
    }

    /// `self` with empty fields taken from `below`.
    fn over(&self, below: &Branding) -> Branding {
        let pick = |a: &String, b: &String| if a.trim().is_empty() { b.clone() } else { a.clone() };
        Branding {
            title_prefix: pick(&self.title_prefix, &below.title_prefix),
            product_name: pick(&self.product_name, &below.product_name),
            logo: pick(&self.logo, &below.logo),
            accent_color: pick(&self.accent_color, &below.accent_color),
            custom_css: pick(&self.custom_css, &below.custom_css),
        }
    }
}

impl From<&ManagedServerBranding> for Branding {
    fn from(m: &ManagedServerBranding) -> Self {
        Self { title_prefix: m.title_prefix.clone(), product_name: m.product_name.clone(), logo: m.logo.clone(), accent_color: m.accent_color.clone(), custom_css: m.custom_css.clone() }
    }
}

fn logo_mime(p: &std::path::Path) -> Option<&'static str> {
    Some(match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    })
}

/// What the web UI needs (public, no secrets).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicBranding {
    pub title_prefix: String,
    pub product_name: String,
    pub accent_color: Option<String>,
    /// `/branding/logo` when a logo is set.
    pub logo_url: Option<String>,
    /// `/branding/custom.css` when custom CSS is set.
    pub css_url: Option<String>,
    /// Changes whenever the branding does (cache busting for the URLs).
    pub revision: String,
}

/// The three layers, shared by the routes and the console handler.
#[derive(Clone, Default)]
pub struct BrandingState {
    inner: Arc<RwLock<Layers>>,
}

#[derive(Default)]
struct Layers {
    cli: Branding,
    saved: Branding,
    managed: Option<Branding>,
}

impl BrandingState {
    /// With the command line / config file layer (validated by the caller).
    pub fn new(cli: Branding) -> Self {
        Self { inner: Arc::new(RwLock::new(Layers { cli, ..Default::default() })) }
    }

    /// Load the administrator's saved branding from the settings database.
    pub async fn load_saved(&self, store: &crate::security::settings_store::SettingsStore) -> Result<(), AppError> {
        if let Some(j) = store.get(SETTINGS_KEY).await? {
            match serde_json::from_str::<Branding>(&j) {
                Ok(b) => self.inner.write().await.saved = b,
                Err(e) => tracing::warn!("Ignoring the saved server branding: {e}"),
            }
        }
        Ok(())
    }

    /// The console's branding (`None` = not managed). Invalid values are refused.
    pub async fn set_managed(&self, m: Option<&ManagedServerBranding>) -> Result<(), String> {
        if let Some(m) = m {
            m.validate()?;
        }
        self.inner.write().await.managed = m.map(Branding::from);
        Ok(())
    }

    pub async fn is_managed(&self) -> bool {
        self.inner.read().await.managed.is_some()
    }

    /// The branding in effect.
    pub async fn effective(&self) -> Branding {
        let l = self.inner.read().await;
        let base = l.saved.over(&l.cli);
        match &l.managed {
            Some(m) => m.over(&base),
            None => base,
        }
    }

    async fn layers(&self) -> (Branding, Branding, Option<Branding>) {
        let l = self.inner.read().await;
        (l.cli.clone(), l.saved.clone(), l.managed.clone())
    }

    async fn save(&self, store: &crate::security::settings_store::SettingsStore, b: Branding) -> Result<(), AppError> {
        b.validate(false)?;
        store.set(SETTINGS_KEY, &serde_json::to_string(&b)?).await?;
        self.inner.write().await.saved = b;
        Ok(())
    }
}

fn revision(b: &Branding) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.hash(&mut h);
    format!("{:x}", h.finish())
}

impl std::hash::Hash for Branding {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        (&self.title_prefix, &self.product_name, &self.logo, &self.accent_color, &self.custom_css).hash(h);
    }
}

pub fn public_view(b: &Branding) -> PublicBranding {
    let rev = revision(b);
    PublicBranding {
        title_prefix: b.title_prefix.clone(),
        product_name: b.product_name.clone(),
        accent_color: Some(b.accent_color.trim().to_string()).filter(|c| !c.is_empty()),
        logo_url: (!b.logo.trim().is_empty()).then(|| format!("/branding/logo?v={rev}")),
        css_url: (!b.custom_css.trim().is_empty()).then(|| format!("/branding/custom.css?v={rev}")),
        revision: rev,
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Brand `index.html`: title (prefix + product name), accent variable,
/// stylesheet link and the public values for the app (`window.__UFOP_BRANDING__`).
pub fn brand_index(index: &str, b: &Branding) -> String {
    if b.is_empty() {
        return index.to_string();
    }
    let mut out = index.to_string();
    if let (Some(s), Some(e)) = (out.find("<title>"), out.find("</title>")) {
        if e > s {
            let current = out[s + 7..e].to_string();
            let name = if b.product_name.trim().is_empty() { current } else { html_escape(&b.product_name) };
            out = format!("{}<title>{}{}</title>{}", &out[..s], html_escape(&b.title_prefix), name, &out[e + 8..]);
        }
    }
    let p = public_view(b);
    let mut head = String::new();
    if let Some(c) = &p.accent_color {
        head.push_str(&format!("<style>:root{{--ufop-accent:{c}}}</style>"));
    }
    if let Some(u) = &p.css_url {
        head.push_str(&format!(r#"<link rel="stylesheet" href="{u}">"#));
    }
    // JSON in a script: `<` is escaped so no value can close the tag.
    let json = serde_json::to_string(&p).unwrap_or_else(|_| "{}".into()).replace('<', "\\u003c");
    head.push_str(&format!("<script>window.__UFOP_BRANDING__={json}</script>"));
    match out.find("</head>") {
        Some(i) => format!("{}{head}{}", &out[..i], &out[i..]),
        None => format!("{head}{out}"),
    }
}

// ── Routes ──────────────────────────────────────────────────────────────

/// `GET /api/v1/branding` (public).
pub async fn public_branding(State(st): State<ServerState>) -> Json<PublicBranding> {
    Json(public_view(&st.branding.effective().await))
}

/// `GET /branding/logo` (public).
pub async fn logo(State(st): State<ServerState>) -> Response {
    let b = st.branding.effective().await;
    let l = b.logo.trim();
    let (mime, bytes) = if l.starts_with("data:") {
        match decode_logo_data_uri(l) {
            Ok((m, d)) => (m, d),
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        }
    } else if !l.is_empty() {
        let p = PathBuf::from(l);
        match (logo_mime(&p), tokio::fs::read(&p).await) {
            (Some(m), Ok(d)) if d.len() <= MAX_BRAND_LOGO => (m.to_string(), d),
            _ => return StatusCode::NOT_FOUND.into_response(),
        }
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // An SVG opened directly must not run scripts on this origin.
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "public, max-age=300".to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'; style-src 'unsafe-inline'; sandbox".to_string()),
        ],
        bytes,
    )
        .into_response()
}

/// `GET /branding/custom.css` (public).
pub async fn custom_css(State(st): State<ServerState>) -> Response {
    let css = st.branding.effective().await.custom_css;
    if css.trim().is_empty() || check_custom_css(&css).is_err() {
        return StatusCode::NOT_FOUND.into_response();
    }
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::X_CONTENT_TYPE_OPTIONS, "nosniff"), (header::CACHE_CONTROL, "public, max-age=300")], css).into_response()
}

#[derive(Serialize)]
struct AdminView {
    effective: Branding,
    /// What `PUT` changes.
    saved: Branding,
    /// From the command line / config file.
    command_line: Branding,
    /// From the management console (changes are refused while set).
    managed: Option<Branding>,
    public: PublicBranding,
}

async fn admin_view(st: &ServerState) -> serde_json::Value {
    let (cli, saved, managed) = st.branding.layers().await;
    let effective = st.branding.effective().await;
    serde_json::to_value(AdminView { public: public_view(&effective), effective, saved, command_line: cli, managed }).unwrap_or_default()
}

/// `GET /api/v1/server/branding` (administrators).
pub async fn get_admin(State(st): State<ServerState>) -> ApiResult {
    Ok(Json(admin_view(&st).await))
}

/// `PUT /api/v1/server/branding` (administrators): replace the saved layer.
pub async fn put_admin(State(st): State<ServerState>, Json(b): Json<Branding>) -> ApiResult {
    if st.branding.is_managed().await {
        return Err(ApiError(AppError::validation("The server branding is managed by your organization's console")));
    }
    st.branding.save(st.auth.settings(), b).await?;
    Ok(Json(admin_view(&st).await))
}

/// The command line layer: `--branding-file` (JSON with these fields) under
/// the individual `--brand-*` flags.
pub fn from_cli(file: Option<&std::path::Path>, flags: Branding) -> Result<Branding, AppError> {
    let base = match file {
        Some(f) => serde_json::from_slice::<Branding>(&std::fs::read(f).map_err(|e| AppError::validation(format!("--branding-file {}: {e}", f.display())))?)
            .map_err(|e| AppError::validation(format!("--branding-file {}: {e}", f.display())))?,
        None => Branding::default(),
    };
    let b = flags.over(&base);
    b.validate(true)?;
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &str = "data:image/png;base64,iVBORw0KGgo=";

    #[tokio::test]
    async fn layers_merge_field_by_field_and_managed_wins() {
        let s = BrandingState::new(Branding { title_prefix: "[cli] ".into(), product_name: "CLI Name".into(), ..Default::default() });
        assert_eq!(s.effective().await.product_name, "CLI Name");
        s.inner.write().await.saved = Branding { product_name: "Saved".into(), accent_color: "#123".into(), ..Default::default() };
        let e = s.effective().await;
        assert_eq!((e.title_prefix.as_str(), e.product_name.as_str(), e.accent_color.as_str()), ("[cli] ", "Saved", "#123"));
        s.set_managed(Some(&ManagedServerBranding { title_prefix: "[ACME] ".into(), ..Default::default() })).await.unwrap();
        let e = s.effective().await;
        assert_eq!((e.title_prefix.as_str(), e.product_name.as_str()), ("[ACME] ", "Saved"));
        assert!(s.set_managed(Some(&ManagedServerBranding { accent_color: "red".into(), ..Default::default() })).await.is_err());
        s.set_managed(None).await.unwrap();
        assert!(!s.is_managed().await);
    }

    #[test]
    fn index_html_is_branded_safely() {
        let idx = "<html><head><title>FileManager</title></head><body></body></html>";
        assert_eq!(brand_index(idx, &Branding::default()), idx);
        let b = Branding { title_prefix: "[<b>ACME</b>] ".into(), accent_color: "#1e66f5".into(), custom_css: "body{color:red}".into(), logo: PNG.into(), product_name: String::new() };
        let out = brand_index(idx, &b);
        assert!(out.contains("<title>[&lt;b&gt;ACME&lt;/b&gt;] FileManager</title>"), "{out}");
        assert!(out.contains("--ufop-accent:#1e66f5"));
        assert!(out.contains(r#"<link rel="stylesheet" href="/branding/custom.css?v="#));
        assert!(out.contains("window.__UFOP_BRANDING__="));
        assert!(!out.contains("<b>"), "script JSON escapes markup");
        let named = brand_index(idx, &Branding { product_name: "ACME Backup".into(), ..Default::default() });
        assert!(named.contains("<title>ACME Backup</title>"));
    }

    #[test]
    fn validation_and_cli_layers() {
        assert!(Branding { logo: "/etc/passwd".into(), ..Default::default() }.validate(false).is_err());
        assert!(Branding { logo: PNG.into(), ..Default::default() }.validate(false).is_ok());
        assert!(Branding { custom_css: "@import url(x)".into(), ..Default::default() }.validate(false).is_err());
        let t = tempfile::tempdir().unwrap();
        let logo = t.path().join("logo.svg");
        std::fs::write(&logo, "<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        let file = t.path().join("branding.json");
        std::fs::write(&file, serde_json::json!({ "title_prefix": "[file] ", "product_name": "File", "logo": logo }).to_string()).unwrap();
        let b = from_cli(Some(&file), Branding { product_name: "Flag".into(), ..Default::default() }).unwrap();
        assert_eq!((b.title_prefix.as_str(), b.product_name.as_str()), ("[file] ", "Flag"));
        assert!(from_cli(Some(&t.path().join("missing.json")), Branding::default()).is_err());
        assert!(from_cli(None, Branding { logo: t.path().join("x.exe").display().to_string(), ..Default::default() }).is_err());
        let p = public_view(&b);
        assert!(p.logo_url.unwrap().starts_with("/branding/logo?v=") && p.css_url.is_none() && p.accent_color.is_none());
    }
}
