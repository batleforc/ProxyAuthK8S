//! A real, ephemeral Kubernetes apiserver for tests.
//!
//! Spawns etcd and kube-apiserver as child processes, hands out a `kube::Client`
//! pointed at them, and tears everything down on drop. This is the only way to
//! check what a mock cannot fake: that the generated CRD — CEL validation rules
//! and cost budget included — is actually accepted by an apiserver, and that
//! admission rejects exactly what `validate()` rejects.
//!
//! Binaries come from `KUBEBUILDER_ASSETS` (see `task envtest:setup`).

#![allow(dead_code)]

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long to wait for the apiserver to answer before giving up.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);

/// Static bearer token wired into `--token-auth-file`.
///
/// The apiserver runs with `AlwaysAllow`, so this only has to authenticate;
/// these tests are about CRD admission and controller behaviour, not RBAC.
const TEST_TOKEN: &str = "envtest-token";

pub struct EnvTest {
    etcd: Child,
    apiserver: Child,
    /// Kept alive so the scratch directory outlives the processes.
    _workdir: TempDir,
    apiserver_url: String,
}

impl EnvTest {
    /// Start etcd and kube-apiserver, or explain why the suite cannot run.
    pub async fn start() -> Result<Self, String> {
        install_crypto_provider();
        let assets = assets_dir()?;
        let etcd_bin = assets.join("etcd");
        let apiserver_bin = assets.join("kube-apiserver");
        for binary in [&etcd_bin, &apiserver_bin] {
            if !binary.exists() {
                return Err(format!(
                    "{} not found; run `task envtest:setup`",
                    binary.display()
                ));
            }
        }

        let workdir = TempDir::new()?;
        let (sa_key, sa_pub) = generate_service_account_keys(workdir.path())?;
        let token_file = workdir.path().join("tokens.csv");
        std::fs::write(
            &token_file,
            format!("{TEST_TOKEN},envtest-admin,uid-1,\"system:masters\"\n"),
        )
        .map_err(|err| format!("could not write the token file: {err}"))?;

        let etcd_client_port = free_port()?;
        let etcd_peer_port = free_port()?;
        let apiserver_port = free_port()?;
        let etcd_url = format!("http://127.0.0.1:{etcd_client_port}");
        let apiserver_url = format!("https://127.0.0.1:{apiserver_port}");

        let etcd = Command::new(&etcd_bin)
            .args([
                "--listen-client-urls",
                &etcd_url,
                "--advertise-client-urls",
                &etcd_url,
                "--listen-peer-urls",
                &format!("http://127.0.0.1:{etcd_peer_port}"),
                "--data-dir",
            ])
            .arg(workdir.path().join("etcd"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("could not start etcd: {err}"))?;

        let apiserver = Command::new(&apiserver_bin)
            .args([
                "--etcd-servers",
                &etcd_url,
                "--bind-address",
                "127.0.0.1",
                "--secure-port",
                &apiserver_port.to_string(),
                "--authorization-mode",
                "AlwaysAllow",
                // The ServiceAccount admission plugin needs a running
                // controller-manager, which envtest does not provide.
                "--disable-admission-plugins",
                "ServiceAccount",
                "--service-cluster-ip-range",
                "10.0.0.0/24",
                "--service-account-issuer",
                "https://kubernetes.default.svc",
            ])
            .arg("--cert-dir")
            .arg(workdir.path().join("certs"))
            .arg("--service-account-key-file")
            .arg(&sa_pub)
            .arg("--service-account-signing-key-file")
            .arg(&sa_key)
            .arg("--token-auth-file")
            .arg(&token_file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("could not start kube-apiserver: {err}"))?;

        let env_test = Self {
            etcd,
            apiserver,
            _workdir: workdir,
            apiserver_url,
        };
        env_test.wait_until_ready().await?;
        Ok(env_test)
    }

    /// Start the apiserver, or skip the test when the binaries are missing.
    ///
    /// Returns `None` after printing why, unless `REQUIRE_ENVTEST` is set —
    /// which CI does, so a broken setup can never silently green the suite.
    pub async fn try_start() -> Option<Self> {
        match Self::start().await {
            Ok(env_test) => Some(env_test),
            Err(err) => {
                assert!(
                    std::env::var("REQUIRE_ENVTEST").is_err(),
                    "REQUIRE_ENVTEST is set but envtest could not start: {err}"
                );
                eprintln!("SKIPPED: {err}");
                None
            }
        }
    }

    async fn wait_until_ready(&self) -> Result<(), String> {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|err| err.to_string())?;
        let healthz = format!("{}/healthz", self.apiserver_url);
        let deadline = Instant::now() + STARTUP_TIMEOUT;

        while Instant::now() < deadline {
            let response = client
                .get(&healthz)
                .bearer_auth(TEST_TOKEN)
                .send()
                .await
                .and_then(|response| response.error_for_status());
            if let Ok(response) = response {
                if response.text().await.unwrap_or_default().trim() == "ok" {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Err(format!(
            "kube-apiserver did not become ready within {STARTUP_TIMEOUT:?}"
        ))
    }

    pub fn url(&self) -> &str {
        &self.apiserver_url
    }

    pub fn token(&self) -> &'static str {
        TEST_TOKEN
    }

    /// A client trusting the apiserver's self-signed certificate.
    pub fn client(&self) -> Result<kube::Client, String> {
        let mut config = kube::Config::new(
            self.apiserver_url
                .parse()
                .map_err(|err| format!("invalid apiserver url: {err}"))?,
        );
        config.accept_invalid_certs = true;
        config.auth_info.token = Some(secrecy::SecretBox::new(TEST_TOKEN.to_string().into()));
        kube::Client::try_from(config).map_err(|err| err.to_string())
    }
}

impl Drop for EnvTest {
    fn drop(&mut self) {
        // Kill the apiserver first: it holds connections to etcd.
        let _ = self.apiserver.kill();
        let _ = self.apiserver.wait();
        let _ = self.etcd.kill();
        let _ = self.etcd.wait();
    }
}

/// The apiserver refuses to start without a service account signing key.
///
/// Shelling out to openssl keeps a 2048-bit RSA generation out of the debug
/// build, where doing it in Rust takes several seconds per test run.
fn generate_service_account_keys(dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    let key = dir.join("sa.key");
    let public = dir.join("sa.pub");

    let generated = Command::new("openssl")
        .arg("genrsa")
        .arg("-out")
        .arg(&key)
        .arg("2048")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| format!("openssl is required by the envtest harness: {err}"))?;
    if !generated.success() {
        return Err("openssl could not generate the service account key".to_string());
    }

    let extracted = Command::new("openssl")
        .arg("rsa")
        .arg("-in")
        .arg(&key)
        .arg("-pubout")
        .arg("-out")
        .arg(&public)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|err| err.to_string())?;
    if !extracted.success() {
        return Err("openssl could not extract the service account public key".to_string());
    }

    Ok((key, public))
}

/// The kube client speaks TLS to the apiserver, so rustls needs its provider.
fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn assets_dir() -> Result<PathBuf, String> {
    std::env::var("KUBEBUILDER_ASSETS")
        .map(PathBuf::from)
        .map_err(|_| "KUBEBUILDER_ASSETS is not set; run `task envtest:run`".to_string())
}

/// Ask the OS for a port, then release it. Racy in principle, fine in practice
/// for a test harness and far simpler than plumbing a port out of the binaries.
fn free_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|err| format!("could not find a port: {err}"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|err| err.to_string())
}

/// Minimal scratch directory with RAII cleanup, to avoid a dev-dependency.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!(
            "proxyauthk8s-envtest-{}-{}",
            std::process::id(),
            next_id()
        ));
        std::fs::create_dir_all(&path).map_err(|err| err.to_string())?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn next_id() -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}
