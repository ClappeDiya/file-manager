//! Client libraries for the `ufop-server` REST API, generated from the
//! OpenAPI document ([`super::openapi::document`]): TypeScript
//! (`sdk/typescript/src/index.ts`, fetch-based, no dependencies) and Python
//! (`sdk/python/ufop_client.py`, standard library only).
//!
//! The generated files are committed; a test regenerates them and fails on
//! any difference (`UPDATE_SDK=1 cargo test --lib sdk` rewrites them), so
//! the clients always match the API. Only the JSON API under `/api/v1` is
//! covered; share pages and OAuth redirects are for browsers.

use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write;

/// One operation of the document, in generation order (by path, then method).
struct Operation<'a> {
    method: &'a str,
    path: &'a str,
    name: String,
    summary: &'a str,
    scope: Option<&'a str>,
    path_params: Vec<&'a str>,
    query: Vec<(&'a str, &'a str, bool, &'a str)>,
    body: Option<&'a Value>,
    response: Option<&'a Value>,
}

fn operations(doc: &Value) -> Vec<Operation<'_>> {
    let mut out = Vec::new();
    let Some(paths) = doc["paths"].as_object() else { return out };
    let sorted: BTreeMap<&String, &Value> = paths.iter().collect();
    for (path, item) in sorted {
        if !path.starts_with("/api/v1/") {
            continue;
        }
        for method in ["get", "post", "put", "patch", "delete"] {
            let Some(op) = item.get(method) else { continue };
            let content = &op["responses"]["200"]["content"];
            let Some(response) = content.get("application/json").map(|c| &c["schema"]) else { continue };
            let id = op["operationId"].as_str().unwrap_or_default();
            let name = id.replacen("_api_v1_", "_", 1).split('_').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("_");
            let params = op["parameters"].as_array().map(Vec::as_slice).unwrap_or_default();
            let path_params = params.iter().filter(|p| p["in"] == "path").filter_map(|p| p["name"].as_str()).collect();
            let query = params
                .iter()
                .filter(|p| p["in"] == "query")
                .map(|p| (p["name"].as_str().unwrap_or_default(), p["schema"]["type"].as_str().unwrap_or("string"), p["required"].as_bool().unwrap_or(false), p["description"].as_str().unwrap_or_default()))
                .collect();
            out.push(Operation {
                method,
                path,
                name,
                summary: op["summary"].as_str().unwrap_or_default(),
                scope: op["x-required-scope"].as_str(),
                path_params,
                query,
                body: op.get("requestBody").map(|b| &b["content"]["application/json"]["schema"]),
                response: Some(response),
            });
        }
    }
    out
}

fn camel(snake: &str) -> String {
    let mut out = String::new();
    for (i, part) in snake.split('_').enumerate() {
        let mut c = part.chars();
        if let Some(f) = c.next() {
            if i == 0 {
                out.push(f.to_ascii_lowercase());
            } else {
                out.push(f.to_ascii_uppercase());
            }
            out.extend(c);
        }
    }
    out
}

fn ref_name(schema: &Value) -> Option<&str> {
    schema["$ref"].as_str().and_then(|r| r.strip_prefix("#/components/schemas/")).map(type_name)
}

/// Schema name in the clients (`Error` would shadow the built-in error type).
fn type_name(name: &str) -> &str {
    if name == "Error" { "ApiError" } else { name }
}

/// A body whose fields are all optional may be left out.
fn body_optional(schema: &Value) -> bool {
    schema["type"] == "object" && schema["required"].as_array().map_or(true, |r| r.is_empty())
}

// ── TypeScript ──

fn ts_type(s: &Value) -> String {
    if let Some(r) = ref_name(s) {
        return r.to_string();
    }
    if let Some(c) = s.get("const") {
        return c.to_string();
    }
    if let Some(e) = s["enum"].as_array() {
        return e.iter().map(Value::to_string).collect::<Vec<_>>().join(" | ");
    }
    if let Some(alts) = s["oneOf"].as_array().or_else(|| s["anyOf"].as_array()) {
        return alts.iter().map(ts_type).collect::<Vec<_>>().join(" | ");
    }
    let types: Vec<&str> = match &s["type"] {
        Value::String(t) => vec![t.as_str()],
        Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    if types.is_empty() {
        return "unknown".into();
    }
    types
        .iter()
        .map(|t| match *t {
            "string" => "string".to_string(),
            "integer" | "number" => "number".into(),
            "boolean" => "boolean".into(),
            "null" => "null".into(),
            "array" => {
                let item = ts_type(&s["items"]);
                if item.contains(' ') { format!("Array<{item}>") } else { format!("{item}[]") }
            }
            "object" => ts_object(s),
            _ => "unknown".into(),
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn ts_key(k: &str) -> String {
    if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.starts_with(|c: char| c.is_ascii_digit()) {
        k.to_string()
    } else {
        format!("{k:?}")
    }
}

fn ts_object(s: &Value) -> String {
    let Some(props) = s["properties"].as_object().filter(|p| !p.is_empty()) else { return "Record<string, unknown>".into() };
    let required: Vec<&str> = s["required"].as_array().map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut fields: Vec<String> = props.iter().map(|(k, v)| format!("{}{}: {}", ts_key(k), if required.contains(&k.as_str()) { "" } else { "?" }, ts_type(v))).collect();
    if s["additionalProperties"] == Value::Bool(true) {
        fields.push("[key: string]: unknown".into());
    }
    format!("{{ {} }}", fields.join("; "))
}

const TS_RUNTIME: &str = r#"/** An error response: `{error_type, message, advice}` and the HTTP status. */
export class UfopError extends Error {
  readonly status: number;
  readonly errorType: string | undefined;
  readonly advice: string | undefined;

  constructor(status: number, message: string, errorType?: string, advice?: string) {
    super(message);
    this.name = "UfopError";
    this.status = status;
    this.errorType = errorType;
    this.advice = advice;
  }
}

export interface ClientOptions {
  /** Access token (from login), automation token or API token (`ufop_pat_…`). */
  token?: string;
  /** A fetch implementation (default: the global one). */
  fetch?: typeof fetch;
  /** Extra headers, e.g. a reverse-proxy pre-authentication header. */
  headers?: Record<string, string>;
}

export class UfopClient {
  readonly baseUrl: string;
  private token: string | undefined;
  private readonly options: ClientOptions;
  private readonly fetchImpl: typeof fetch;

  constructor(baseUrl: string, options: ClientOptions = {}) {
    this.baseUrl = baseUrl;
    this.options = options;
    this.token = options.token;
    this.fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
  }

  setToken(token: string | undefined): void {
    this.token = token;
  }

  /** Sign in (account `username`, default admin) and use the access token from then on. */
  async login(password: string, username?: string): Promise<Session> {
    const session = await this.postAuthLogin(username ? { username, password } : { password });
    this.token = session.access_token;
    return session;
  }

  private async call<T>(method: string, path: string, query?: Record<string, unknown>, body?: unknown): Promise<T> {
    const url = new URL(path, this.baseUrl.endsWith("/") ? this.baseUrl : `${this.baseUrl}/`);
    for (const [k, v] of Object.entries(query ?? {})) if (v !== undefined && v !== null) url.searchParams.set(k, String(v));
    const headers: Record<string, string> = { Accept: "application/json", ...this.options.headers };
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    if (body !== undefined) headers["Content-Type"] = "application/json";
    const res = await this.fetchImpl(url.toString(), { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
    const text = await res.text();
    const data = text ? JSON.parse(text) : undefined;
    if (!res.ok) throw new UfopError(res.status, data?.message ?? res.statusText, data?.error_type, data?.advice);
    return data as T;
  }
"#;

/// The TypeScript client.
pub fn typescript(doc: &Value) -> String {
    let mut s = String::from("// Generated from the ufop-server OpenAPI document by src-tauri/src/server/sdk.rs. Do not edit.\n// Regenerate: UPDATE_SDK=1 cargo test --lib sdk\n\n");
    let _ = writeln!(s, "/** API version this client was generated from. */\nexport const API_VERSION = {:?};\n", doc["info"]["version"].as_str().unwrap_or_default());
    if let Some(schemas) = doc["components"]["schemas"].as_object() {
        for (name, schema) in schemas.iter().collect::<BTreeMap<_, _>>() {
            if let Some(d) = schema["description"].as_str() {
                let _ = writeln!(s, "/** {d} */");
            }
            let _ = writeln!(s, "export type {} = {};\n", type_name(name), ts_type(schema));
        }
    }
    s.push_str(TS_RUNTIME);
    for op in operations(doc) {
        let mut args: Vec<String> = op.path_params.iter().map(|p| format!("{}: string", camel(p))).collect();
        if let Some(b) = op.body {
            args.push(format!("body: {}{}", ts_type(b), if body_optional(b) && op.query.iter().all(|q| !q.2) { " = {}" } else { "" }));
        }
        if !op.query.is_empty() {
            let fields: Vec<String> = op.query.iter().map(|(n, t, req, _)| format!("{}{}: {}", ts_key(n), if *req { "" } else { "?" }, if *t == "integer" { "number" } else { t })).collect();
            let all_optional = op.query.iter().all(|q| !q.2);
            args.push(format!("query: {{ {} }}{}", fields.join("; "), if all_optional { " = {}" } else { "" }));
        }
        let mut path = op.path.to_string();
        for p in &op.path_params {
            path = path.replace(&format!("{{{p}}}"), &format!("${{encodeURIComponent({})}}", camel(p)));
        }
        let ret = op.response.map(ts_type).unwrap_or_else(|| "unknown".into());
        let doc_line = match op.scope {
            Some(scope) => format!("{} (API tokens: `{scope}` scope)", op.summary),
            None => op.summary.to_string(),
        };
        let _ = write!(
            s,
            "\n  /** {doc_line}. `{} {}` */\n  {}({}): Promise<{ret}> {{\n    return this.call(\"{}\", `{path}`, {}, {});\n  }}\n",
            op.method.to_uppercase(),
            op.path,
            camel(&op.name),
            args.join(", "),
            op.method.to_uppercase(),
            if op.query.is_empty() { "undefined" } else { "query" },
            if op.body.is_some() { "body" } else { "undefined" },
        );
    }
    s.push_str("}\n");
    s
}

// ── Python ──

const PY_KEYWORDS: &[&str] = &["from", "import", "class", "def", "global", "in", "is", "not", "or", "and", "pass", "return", "lambda", "with", "yield", "async", "await", "type", "id"];

fn py_name(n: &str) -> String {
    if PY_KEYWORDS.contains(&n) { format!("{n}_") } else { n.to_string() }
}

fn py_type(s: &Value) -> String {
    if let Some(r) = ref_name(s) {
        return format!("\"{r}\"");
    }
    let types: Vec<&str> = match &s["type"] {
        Value::String(t) => vec![t.as_str()],
        Value::Array(ts) => ts.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    let mut out: Vec<String> = types
        .iter()
        .map(|t| match *t {
            "string" => "str".to_string(),
            "integer" => "int".into(),
            "number" => "float".into(),
            "boolean" => "bool".into(),
            "null" => "None".into(),
            "array" => format!("List[{}]", py_type(&s["items"])),
            _ => "Dict[str, Any]".into(),
        })
        .collect();
    if out.is_empty() {
        return "Any".into();
    }
    if out.len() == 1 {
        return out.remove(0);
    }
    format!("Union[{}]", out.join(", "))
}

const PY_RUNTIME: &str = r#"

class UfopError(Exception):
    """An error response: `{error_type, message, advice}` and the HTTP status."""

    def __init__(self, status: int, message: str, error_type: Optional[str] = None, advice: Optional[str] = None):
        super().__init__(message)
        self.status = status
        self.error_type = error_type
        self.advice = advice


class UfopClient:
    """ufop-server REST client (standard library only).

    `token` is an access token (from `login`), an automation token or an API
    token (`ufop_pat_…`). `headers` adds headers to every request.
    """

    def __init__(self, base_url: str, token: Optional[str] = None, timeout: float = 30.0, headers: Optional[Dict[str, str]] = None):
        self.base_url = base_url.rstrip("/")
        self.token = token
        self.timeout = timeout
        self.headers = dict(headers or {})

    def login(self, password: str, username: Optional[str] = None) -> Dict[str, Any]:
        """Sign in (account `username`, default admin) and use the access token from then on."""
        session = self.post_auth_login({"password": password, **({"username": username} if username else {})})
        self.token = session["access_token"]
        return session

    def _call(self, method: str, path: str, query: Optional[Dict[str, Any]] = None, body: Any = None) -> Any:
        url = self.base_url + path
        if query:
            q = {k: ("true" if v is True else "false" if v is False else v) for k, v in query.items() if v is not None}
            if q:
                url += "?" + urllib.parse.urlencode(q)
        headers = {"Accept": "application/json", **self.headers}
        if self.token:
            headers["Authorization"] = "Bearer " + self.token
        data = None
        if body is not None:
            data = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        req = urllib.request.Request(url, data=data, method=method, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as res:
                text = res.read().decode()
                return json.loads(text) if text else None
        except urllib.error.HTTPError as e:
            text = e.read().decode(errors="replace")
            try:
                err = json.loads(text)
            except ValueError:
                err = {}
            raise UfopError(e.code, err.get("message") or e.reason, err.get("error_type"), err.get("advice")) from None
"#;

/// The Python client.
pub fn python(doc: &Value) -> String {
    let mut s = String::from("# Generated from the ufop-server OpenAPI document by src-tauri/src/server/sdk.rs. Do not edit.\n# Regenerate: UPDATE_SDK=1 cargo test --lib sdk\n\"\"\"Client for the ufop-server REST API.\"\"\"\n\nimport json\nimport urllib.error\nimport urllib.parse\nimport urllib.request\nfrom typing import Any, Dict, List, Optional, Union\n\n");
    let _ = writeln!(s, "API_VERSION = {:?}", doc["info"]["version"].as_str().unwrap_or_default());
    if let Some(schemas) = doc["components"]["schemas"].as_object() {
        for name in schemas.keys().collect::<std::collections::BTreeSet<_>>() {
            let _ = writeln!(s, "{} = Dict[str, Any]", type_name(name));
        }
    }
    s.push_str(PY_RUNTIME);
    for op in operations(doc) {
        let mut args = vec!["self".to_string()];
        args.extend(op.path_params.iter().map(|p| format!("{}: str", py_name(p))));
        // Optional only when no required query parameter follows it.
        let optional_body = op.body.is_some_and(body_optional) && op.query.iter().all(|q| !q.2);
        if let Some(b) = op.body {
            args.push(if optional_body { format!("body: Optional[{}] = None", py_type(b)) } else { format!("body: {}", py_type(b)) });
        }
        // Required query parameters first (Python needs defaults last).
        let mut query_args: Vec<&(&str, &str, bool, &str)> = op.query.iter().collect();
        query_args.sort_by_key(|q| !q.2);
        for (n, t, required, _) in query_args {
            let t = match *t {
                "integer" => "int",
                "boolean" => "bool",
                _ => "str",
            };
            args.push(if *required { format!("{}: {t}", py_name(n)) } else { format!("{}: Optional[{t}] = None", py_name(n)) });
        }
        let mut path = op.path.to_string();
        for p in &op.path_params {
            path = path.replace(&format!("{{{p}}}"), &format!("{{urllib.parse.quote({}, safe='')}}", py_name(p)));
        }
        let query = if op.query.is_empty() { "None".to_string() } else { format!("{{{}}}", op.query.iter().map(|(n, ..)| format!("{n:?}: {}", py_name(n))).collect::<Vec<_>>().join(", ")) };
        let doc_line = match op.scope {
            Some(scope) => format!("{} (API tokens: `{scope}` scope). `{} {}`", op.summary, op.method.to_uppercase(), op.path),
            None => format!("{}. `{} {}`", op.summary, op.method.to_uppercase(), op.path),
        };
        let ret = op.response.map(py_type).unwrap_or_else(|| "Any".into());
        let _ = write!(
            s,
            "\n    def {}({}) -> {ret}:\n        \"\"\"{}\"\"\"\n        return self._call({:?}, f\"{path}\", {query}, {})\n",
            op.name,
            args.join(", "),
            doc_line.replace('"', "'"),
            op.method.to_uppercase(),
            if optional_body { "body if body is not None else {}" } else if op.body.is_some() { "body" } else { "None" },
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sdk_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sdk")
    }

    /// The committed clients are what the generator produces now.
    #[test]
    fn committed_clients_match_the_api() {
        let doc = super::super::openapi::document();
        for (rel, text) in [("typescript/src/index.ts", typescript(&doc)), ("python/ufop_client.py", python(&doc))] {
            let path = sdk_dir().join(rel);
            if std::env::var("UPDATE_SDK").is_ok() {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, &text).unwrap();
            }
            let committed = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(committed == text, "{} is out of date: run UPDATE_SDK=1 cargo test --lib sdk", path.display());
        }
    }

    #[test]
    fn names_types_and_coverage() {
        let doc = super::super::openapi::document();
        let ops = operations(&doc);
        assert!(ops.iter().all(|o| o.path.starts_with("/api/v1/")));
        let names: std::collections::BTreeSet<&str> = ops.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names.len(), ops.len(), "unique method names");
        assert!(names.contains("get_jobs_id") && names.contains("post_jobs_id_run") && names.contains("delete_shares_id"), "{names:?}");
        let ts = typescript(&doc);
        assert!(ts.contains("getJobsId(id: string): Promise<BackupJob>"));
        assert!(ts.contains("getJobsIdSearch(id: string, query: { q: string; version?: number; source?: string }): Promise<Record<string, unknown>>"), "required query → required object");
        assert!(ts.contains("export type Scope = \"read\" | \"run\" | \"write\" | \"admin\";"));
        assert!(ts.contains("`/api/v1/jobs/${encodeURIComponent(id)}/run`"));
        let py = python(&doc);
        assert!(py.contains("def get_jobs_id_search(self, id_: str, q: str, version: Optional[int] = None, source: Optional[str] = None)"));
        assert!(py.contains("(API tokens: `run` scope)"));
        assert_eq!(camel("post_auth_api_tokens"), "postAuthApiTokens");
    }
}
