//! The bench `it:up` mounted: its run directory, an HTTP client that reaches
//! it, and admin access for setup and checks.

use std::{
    io::Write as _,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
};

pub const HOST: &str = "proxyauth.k8s.localhost";
pub const DEX_HOST: &str = "dex.k8s.localhost";
/// The namespace `it:up` installs the chart, Dex and Redis in.
pub const STACK_NS: &str = "proxyauth";
/// The back's Service, as the chart names it for the `proxyauth` release.
pub const BACK_SERVICE: &str = "proxyauth-proxyauthk8s-back";

pub struct Bench {
    pub root: PathBuf,
    pub run: PathBuf,
    pub port: u16,
}

impl Bench {
    /// The bench of this run. Panics with the fix when `it:up` was not run.
    pub fn get() -> &'static Bench {
        static BENCH: OnceLock<Bench> = OnceLock::new();
        BENCH.get_or_init(|| {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let root = root.canonicalize().expect("repository root");
            let run = std::env::var_os("IT_DIR")
                .map_or_else(|| root.join(".it/run"), |dir| root.join(dir));
            assert!(
                run.join("admin.kubeconfig").is_file(),
                "no bench at {}: run `task it:up` first",
                run.display()
            );
            let port = std::env::var("IT_HTTPS_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8443);
            Bench { root, run, port }
        })
    }

    pub fn ca_path(&self) -> PathBuf {
        self.run.join("ca/rootCA.pem")
    }

    pub fn admin_kubeconfig(&self) -> PathBuf {
        self.run.join("admin.kubeconfig")
    }

    /// `https://proxyauth.k8s.localhost:<port>`, what every client is given.
    pub fn base_url(&self) -> String {
        format!("https://{HOST}:{}", self.port)
    }

    pub fn dex_url(&self) -> String {
        format!("https://{DEX_HOST}:{}", self.port)
    }

    /// A client that trusts only the run's CA, resolves both bench names to
    /// the published port, and follows no redirect (tests look at them).
    pub fn http(&self) -> reqwest::Client {
        let ca = std::fs::read(self.ca_path()).expect("run CA");
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(&ca).expect("run CA is PEM")])
            .resolve(HOST, addr)
            .resolve(DEX_HOST, addr)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client")
    }

    /// A request to `path` of a proxied cluster, as a raw client would send
    /// it (for what kubectl cannot produce), with the user's cluster token.
    pub fn proxied(
        &self,
        method: reqwest::Method,
        ns: &str,
        cluster: &str,
        path: &str,
        token: &str,
    ) -> reqwest::RequestBuilder {
        self.http()
            .request(
                method,
                format!("{}/clusters/{ns}/{cluster}{path}", self.base_url()),
            )
            .bearer_auth(token)
    }

    /// `kubectl` as the cluster admin: setup and checks only.
    pub fn admin(&self, args: &[&str]) -> std::process::Output {
        self.admin_with_stdin(args, None)
    }

    pub fn admin_with_stdin(&self, args: &[&str], stdin: Option<&str>) -> std::process::Output {
        let mut child = Command::new("kubectl")
            .arg("--kubeconfig")
            .arg(self.admin_kubeconfig())
            .args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("kubectl is on PATH");
        if let Some(input) = stdin {
            child
                .stdin
                .take()
                .expect("piped stdin")
                .write_all(input.as_bytes())
                .expect("write stdin");
        }
        child.wait_with_output().expect("kubectl ran")
    }

    /// `kubectl apply -f -` as the admin; panics with kubectl's message.
    pub fn apply(&self, yaml: &str) {
        let out = self.admin_with_stdin(&["apply", "-f", "-"], Some(yaml));
        assert!(
            out.status.success(),
            "admin apply failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A fresh namespace for one test, deleted when the guard drops.
    pub fn namespace(&self, test: &str) -> Namespace {
        let name = namespace_name(test);
        // A leftover from an interrupted run would make the test see old state.
        self.admin(&["delete", "namespace", &name, "--ignore-not-found", "--wait"]);
        let out = self.admin(&["create", "namespace", &name]);
        assert!(
            out.status.success(),
            "create namespace {name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Namespace { name }
    }
}

/// `it-<test>`, lowercased and cut to a valid DNS label.
pub fn namespace_name(test: &str) -> String {
    let mut name: String = format!("it-{test}")
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    name.truncate(63);
    name.trim_end_matches('-').to_string()
}

pub struct Namespace {
    pub name: String,
}

impl Drop for Namespace {
    fn drop(&mut self) {
        if std::env::var_os("IT_KEEP").is_none() {
            Bench::get().admin(&["delete", "namespace", &self.name, "--wait=false"]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::namespace_name;

    #[test]
    fn namespace_names_are_dns_labels() {
        assert_eq!(namespace_name("rfc0009::p2_Login"), "it-rfc0009--p2-login");
        assert_eq!(namespace_name(&"x".repeat(80)).len(), 63);
        assert_eq!(namespace_name("ends_"), "it-ends");
    }
}
