//! The real `kubectl-proxyauth` binary against a mocked ProxyAuthK8S: the
//! paths unit tests cannot reach because they read the process environment,
//! the terminal or spawn a browser (`main`, `get-token` reading
//! `KUBERNETES_EXEC_INFO`, the SSO logins through the callback port and a
//! browser program, the token prompt on stdin).
//!
//! The "browser" is `curl -L`: it opens the URL the CLI hands it and follows
//! the provider's redirect to the CLI's loopback listener, as a browser would.
//! Each run gets its own `HOME`, kubeconfig and keyring service, cleared when
//! the test ends; the session bus is pointed nowhere so the keyring is the
//! kernel's (keyutils), never the desktop's Secret Service.

#![expect(
    clippy::expect_used,
    reason = "test harness: a panic is a failed test, with its message"
)]

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use tempfile::TempDir;
use tokio::{io::AsyncWriteExt, process::Command};
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{header, method, path, query_param},
};

const NS: &str = "team-a";
const SERVER_TOKEN: &str = "server-token";
/// What the mocked server answers `/api/v1/auth/token` with (SSO login).
const SSO_ACCESS: &str = "sso-access";

/// One user's CLI: its `HOME`, kubeconfig, keyring service and callback port.
struct User {
    home: TempDir,
    port: u16,
}

/// What a run printed and how it exited.
#[derive(Debug)]
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Run {
    fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|addr| addr.port())
        .expect("a free loopback port")
}

impl User {
    fn new() -> Self {
        User {
            home: tempfile::tempdir().expect("a scratch HOME"),
            port: free_port(),
        }
    }

    fn kubeconfig(&self) -> PathBuf {
        self.home.path().join(".kube").join("config")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kubectl_proxyauth"));
        command
            .args(args)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.home.path())
            .env("KUBECONFIG", self.kubeconfig())
            // Unique per user: never another test's (or the developer's) tokens.
            .env(
                "PROXYAUTH_KEYRING_SERVICE",
                format!("proxyauthk8s-it-{}", self.home.path().display()),
            )
            // Unset is not enough: zbus would still find $XDG_RUNTIME_DIR/bus.
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent")
            .env("PROXYAUTH_CALLBACK_PORT", self.port.to_string())
            .env("NO_COLOR", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // coverage: forward the profile path so `cargo llvm-cov` counts the
        // child's lines.
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        command
    }

    async fn run_with(&self, args: &[&str], env: &[(&str, &str)], stdin: &str) -> Run {
        let mut command = self.command(args);
        for (key, value) in env {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("the CLI starts");
        if let Some(mut input) = child.stdin.take() {
            input
                .write_all(stdin.as_bytes())
                .await
                .expect("stdin is writable");
        }
        let out = child.wait_with_output().await.expect("the CLI exits");
        Run {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    async fn run(&self, args: &[&str]) -> Run {
        self.run_with(args, &[], "").await
    }
}

/// The tokens a test stored live in the developer's real keyring (keyutils):
/// clear them with the run, whatever its outcome.
impl Drop for User {
    fn drop(&mut self) {
        let mut clear = self.command(&["config", "clear", "--all"]);
        match clear.as_std_mut().output() {
            Ok(out) if out.status.success() => {}
            other => eprintln!("could not clear the test keyring entries: {other:?}"),
        }
    }
}

/// `--browser curl …`: follows every redirect, discards the pages.
const CURL_BROWSER: [&str; 10] = [
    "--browser",
    "curl",
    "--browser-arg",
    "-sSL",
    "--browser-arg",
    "-o",
    "--browser-arg",
    "/dev/null",
    "--browser-arg",
    "{url}",
];

fn with_browser<'a>(args: &[&'a str]) -> Vec<&'a str> {
    args.iter().copied().chain(CURL_BROWSER).collect()
}

/// `GET /api/v1/clusters` for `token`: `plain` (no SSO) and `sso` in `team-a`.
async fn mount_clusters(server: &MockServer, token: &str) {
    Mock::given(method("GET"))
        .and(path("/api/v1/clusters"))
        .and(header("authorization", format!("Bearer {token}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "clusters": [
                {"enabled": true, "is_reachable": true, "name": "plain", "namespace": NS, "sso_enabled": false},
                {"enabled": true, "is_reachable": true, "name": "sso", "namespace": NS, "sso_enabled": true},
            ]
        })))
        .mount(server)
        .await;
}

/// The server's `/api/v1/auth/login`, as its IdP would end it: a redirect to
/// the CLI's `redirect_uri` with a code and the CLI's own `state` (or the
/// provider's `error`).
struct LoginRedirect {
    error: Option<&'static str>,
}

impl Respond for LoginRedirect {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let query = |name: &str| {
            request
                .url
                .query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
                .unwrap_or_default()
        };
        let location = match self.error {
            Some(error) => format!("{}?error={error}", query("redirect_uri")),
            None => format!(
                "{}?code=one-time&state={}",
                query("redirect_uri"),
                query("state")
            ),
        };
        ResponseTemplate::new(302).insert_header("location", location.as_str())
    }
}

/// A server whose SSO login hands out [`SSO_ACCESS`] for the code `one-time`.
async fn mount_server_sso(server: &MockServer, error: Option<&'static str>) {
    Mock::given(method("GET"))
        .and(path("/api/v1/auth/login"))
        .respond_with(LoginRedirect { error })
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": SSO_ACCESS,
            "token_type": "Bearer",
            "expires_in": 3600,
            "refresh_token": "sso-refresh",
        })))
        .mount(server)
        .await;
    mount_clusters(server, SSO_ACCESS).await;
}

/// The OIDC login of `team-a/sso`: the server hands out the provider URL, the
/// provider redirects to the CLI's port, the server exchanges the code.
async fn mount_cluster_sso(server: &MockServer, port: u16, id_token: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/clusters/{NS}/sso/auth/login")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!("{}/idp/authorize", server.uri()))
                .insert_header("content-type", "text/plain"),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/idp/authorize"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "location",
            format!("http://localhost:{port}/?code=idp-code&state=idp-state").as_str(),
        ))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/clusters/{NS}/sso/auth/callback")))
        .and(query_param("code", "idp-code"))
        .and(query_param("state", "idp-state"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "idp-access",
            "cluster_url": format!("{}/clusters/{NS}/sso", server.uri()),
            "id_token": id_token,
            "refresh_token": "idp-refresh",
            "subject": "alice",
        })))
        .mount(server)
        .await;
}

/// `KUBERNETES_EXEC_INFO` as kubectl sets it for a proxied cluster.
fn exec_info(server: &MockServer, cluster: &str) -> String {
    serde_json::json!({
        "apiVersion": "client.authentication.k8s.io/v1",
        "kind": "ExecCredential",
        "spec": { "cluster": { "server": format!("{}/clusters/{NS}/{cluster}", server.uri()) } }
    })
    .to_string()
}

fn token_of(credential: &Run) -> String {
    let value: serde_json::Value =
        serde_json::from_str(credential.stdout.trim()).expect("get-token prints JSON");
    value
        .get("status")
        .and_then(|status| status.get("token"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[tokio::test]
async fn an_sso_server_login_then_an_oidc_cluster_login_serve_kubectl() {
    let server = MockServer::start().await;
    let user = User::new();
    mount_server_sso(&server, None).await;
    mount_cluster_sso(&server, user.port, "alice-id-token").await;
    let url = server.uri();

    // 1. SSO to the server: the browser follows the IdP back to the CLI.
    let login = user
        .run(&with_browser(&["login", "--server-url", &url]))
        .await;
    assert!(login.ok(), "{login:?}");
    let status = user.run(&["status", "-f", "json"]).await;
    assert!(
        status.ok() && status.stdout.contains("\"login\": \"sso\""),
        "{status:?}"
    );
    let get = user.run(&["get"]).await;
    assert!(get.ok() && get.stdout.contains("sso"), "{get:?}");

    // 2. OIDC login to the SSO cluster: id_token stored, context written.
    let cluster = user.run(&with_browser(&["-n", NS, "login", "sso"])).await;
    assert!(cluster.ok(), "{cluster:?}");
    let kubeconfig = read(&user.kubeconfig());
    assert!(
        kubeconfig.contains(&format!("{NS}-sso-context")) && kubeconfig.contains("get-token"),
        "{kubeconfig}"
    );

    // 3. kubectl's side: get-token hands out the id_token.
    let credential = user
        .run_with(
            &["get-token"],
            &[("KUBERNETES_EXEC_INFO", &exec_info(&server, "sso"))],
            "",
        )
        .await;
    assert!(credential.ok(), "{credential:?}");
    assert_eq!(token_of(&credential), "alice-id-token");

    // 4. Logging out of the cluster forgets the id_token.
    let logout = user.run(&["-n", NS, "logout", "sso"]).await;
    assert!(logout.ok(), "{logout:?}");
    let after = user
        .run_with(
            &["get-token"],
            &[("KUBERNETES_EXEC_INFO", &exec_info(&server, "sso"))],
            "",
        )
        .await;
    assert_eq!(after.code, Some(1), "{after:?}");
}

#[tokio::test]
async fn a_cluster_token_typed_at_the_prompt_is_stored() {
    let server = MockServer::start().await;
    mount_clusters(&server, SERVER_TOKEN).await;
    let user = User::new();
    let url = server.uri();

    let login = user
        .run(&["login", "--server-url", &url, "--token", SERVER_TOKEN])
        .await;
    assert!(login.ok(), "{login:?}");
    assert!(
        login.stderr.contains("--token on the command line"),
        "{login:?}"
    );

    // A cluster without SSO and no --token: the token is asked for.
    let cluster = user
        .run_with(&["-n", NS, "login", "plain"], &[], "typed-token\n")
        .await;
    assert!(cluster.ok(), "{cluster:?}");
    assert!(
        cluster.stdout.contains("Enter cluster token"),
        "{cluster:?}"
    );
    let credential = user
        .run_with(
            &["get-token", "plain"],
            &[("KUBERNETES_EXEC_INFO", &exec_info(&server, "plain"))],
            "",
        )
        .await;
    assert_eq!(token_of(&credential), "typed-token", "{credential:?}");

    // Nothing typed: refused, nothing stored.
    let empty = user
        .run_with(&["-n", NS, "login", "plain"], &[], "\n")
        .await;
    assert_eq!(empty.code, Some(1), "{empty:?}");
    assert!(empty.stderr.contains("requires a token"), "{empty:?}");
}

#[tokio::test]
async fn an_sso_login_the_provider_denies_is_refused_and_stores_nothing() {
    let server = MockServer::start().await;
    mount_server_sso(&server, Some("access_denied")).await;
    let user = User::new();

    let login = user
        .run(&with_browser(&["login", "--server-url", &server.uri()]))
        .await;
    assert_eq!(login.code, Some(1), "{login:?}");
    assert!(login.stderr.contains("access_denied"), "{login:?}");
    let status = user.run(&["status"]).await;
    assert_eq!(status.code, Some(1), "{status:?}");
    assert!(status.stderr.contains("ERR000021"), "{status:?}");
}

#[tokio::test]
async fn an_oidc_cluster_login_whose_code_exchange_fails_is_refused() {
    let server = MockServer::start().await;
    let user = User::new();
    mount_server_sso(&server, None).await;
    // The provider redirects back, but the server cannot redeem the code.
    Mock::given(method("GET"))
        .and(path(format!("/clusters/{NS}/sso/auth/callback")))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    mount_cluster_sso(&server, user.port, "never-issued").await;
    let login = user
        .run(&with_browser(&["login", "--server-url", &server.uri()]))
        .await;
    assert!(login.ok(), "{login:?}");

    let cluster = user.run(&with_browser(&["-n", NS, "login", "sso"])).await;
    assert_eq!(cluster.code, Some(1), "{cluster:?}");
    assert!(cluster.stderr.contains("ERR000015"), "{cluster:?}");
    assert!(
        !read(&user.kubeconfig()).contains("sso-context"),
        "no context for a failed login"
    );
    let credential = user
        .run_with(
            &["get-token"],
            &[("KUBERNETES_EXEC_INFO", &exec_info(&server, "sso"))],
            "",
        )
        .await;
    assert_eq!(credential.code, Some(1), "{credential:?}");
}

#[tokio::test]
async fn get_token_run_outside_kubectl_is_refused() {
    let server = MockServer::start().await;
    mount_clusters(&server, SERVER_TOKEN).await;
    let user = User::new();
    let login = user
        .run(&[
            "login",
            "--server-url",
            &server.uri(),
            "--token",
            SERVER_TOKEN,
        ])
        .await;
    assert!(login.ok(), "{login:?}");

    let out = user.run(&["get-token", "plain"]).await;
    assert_eq!(out.code, Some(1), "{out:?}");
    assert!(out.stdout.is_empty(), "no credential on stdout: {out:?}");
    assert!(out.stderr.contains("ERR000018"), "{out:?}");
}

#[tokio::test]
async fn a_config_command_refused_prints_its_error_once() {
    let user = User::new();
    for args in [
        &["config", "use-server", "https://unknown.example"][..],
        &["config", "clear"][..],
        &["config", "clear", "-s", "https://unknown.example"][..],
    ] {
        let out = user.run(args).await;
        assert_eq!(out.code, Some(1), "{args:?}: {out:?}");
        assert_eq!(
            out.stderr.matches("error: ERR").count(),
            1,
            "{args:?}: {out:?}"
        );
        assert!(!out.stderr.contains("WARN"), "{args:?}: {out:?}");
    }
}

#[tokio::test]
async fn a_malformed_cli_config_fails_cleanly_before_any_command() {
    let user = User::new();
    let config = user.home.path().join("broken.yaml");
    std::fs::write(&config, "servers: [not, a, map").expect("scratch file");
    let out = user
        .run(&["-p", config.to_str().unwrap_or_default(), "status"])
        .await;
    assert_eq!(out.code, Some(1), "{out:?}");
    assert!(
        out.stderr.starts_with("Error: "),
        "no panic, a message: {out:?}"
    );
}
