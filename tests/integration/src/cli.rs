//! The real `kubectl-proxyauth` and `kubectl`, run in a throwaway `HOME`
//! (RFC 0009 §5.2). The test plays the browser: it reads the authorization
//! URL the CLI prints, fills Dex's form over HTTP and follows the redirect to
//! the CLI's own loopback listener.

use std::{path::PathBuf, process::Stdio, time::Duration};

use tempfile::TempDir;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
};
use url::Url;

use crate::{Bench, Persona};

/// Loopback callback ports the suite's CLI logins use (Dex accepts any port
/// for the public `kube_login` client; a small range keeps them recognisable).
const CALLBACK_PORTS: std::ops::Range<u16> = 18_000..18_008;

/// A callback port free right now. nextest runs each test in its own
/// process, so no counter is shared: start from an offset taken from the
/// test's namespace and take the first port that binds. shortcut: a port can
/// still be taken between this check and the CLI's own bind; widen the range
/// if that ever shows up.
fn callback_port(seed: &str) -> u16 {
    let span = CALLBACK_PORTS.end - CALLBACK_PORTS.start;
    let offset = seed.bytes().fold(0u16, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(u16::from(b))
    });
    (0..span)
        .map(|i| CALLBACK_PORTS.start + (offset.wrapping_add(i)) % span)
        .find(|port| std::net::TcpListener::bind(("127.0.0.1", *port)).is_ok())
        .expect("every callback port of the range is in use")
}

/// What a command printed and how it exited.
#[derive(Debug)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// A persona's CLI state: its `HOME`, kubeconfig and CLI configuration.
pub struct Session {
    pub persona: Persona,
    home: TempDir,
    bin: TempDir,
}

impl Session {
    /// A logged-in server session: `kubectl proxyauth login --server-url …
    /// --token <dashboard token>`, as the dashboard tells a user to run.
    pub async fn server_login(persona: &Persona) -> Session {
        let bench = Bench::get();
        let home = tempfile::tempdir().expect("temp HOME");
        let bin = tempfile::tempdir().expect("temp PATH dir");
        // kubectl finds its plugin as `kubectl-proxyauth` on PATH.
        std::os::unix::fs::symlink(cli_binary(), bin.path().join("kubectl-proxyauth"))
            .expect("link kubectl-proxyauth");
        let session = Session {
            persona: persona.clone(),
            home,
            bin,
        };
        let token = persona.dashboard_token().await;
        let ca = bench.ca_path();
        let out = session
            .cli(&[
                "login",
                "--server-url",
                &bench.base_url(),
                "--certificate-authority",
                ca.to_str().expect("utf-8 path"),
                "--token",
                &token,
            ])
            .await;
        assert!(
            out.success(),
            "server login of {} failed: {out:?}",
            persona.name
        );
        session
    }

    /// `kubectl proxyauth login <cluster> -n <ns>` through the SSO flow.
    /// Returns the CLI's output; `Ok` when it exited 0.
    pub async fn cluster_login(&self, ns: &str, cluster: &str) -> Result<Output, Output> {
        let port = callback_port(ns);
        let mut child = self
            .command(&cli_binary())
            .args(["login", cluster, "--namespace", ns])
            .env("PROXYAUTH_BROWSER", "none")
            .env("PROXYAUTH_CALLBACK_PORT", port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn kubectl-proxyauth");

        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = BufReader::new(child.stderr.take().expect("piped stderr")).lines();
        let mut printed = String::new();
        let mut url = None;
        let read_url = async {
            while let Ok(Some(line)) = stderr.next_line().await {
                printed.push_str(&line);
                printed.push('\n');
                if let Some(found) = authorization_url(&line) {
                    url = Some(found);
                    break;
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(30), read_url)
            .await
            .expect("the CLI printed nothing for 30 s");

        if let Some(url) = url {
            play_browser(&self.persona, &url, port).await;
        }
        // Drain the rest so the CLI never blocks on a full pipe.
        let rest = async {
            while let Ok(Some(line)) = stderr.next_line().await {
                printed.push_str(&line);
                printed.push('\n');
            }
        };
        let mut out = String::new();
        let (status, _, _) = tokio::time::timeout(Duration::from_secs(60), async {
            tokio::join!(child.wait(), rest, stdout.read_to_string(&mut out))
        })
        .await
        .expect("the CLI did not exit within 60 s");
        let output = Output {
            code: status.expect("CLI status").code(),
            stdout: out,
            stderr: printed,
        };
        if output.success() {
            Ok(output)
        } else {
            Err(output)
        }
    }

    /// Run `kubectl-proxyauth` itself.
    pub async fn cli(&self, args: &[&str]) -> Output {
        run(self.command(&cli_binary()).args(args)).await
    }

    /// The real `kubectl`, with this session's kubeconfig: credentials come
    /// from the `get-token` plugin, called by kubectl itself.
    pub async fn kubectl(&self, args: &[&str]) -> Output {
        run(self.command(&PathBuf::from("kubectl")).args(args)).await
    }

    fn command(&self, program: &PathBuf) -> Command {
        let mut command = Command::new(program);
        let path = std::env::var("PATH").unwrap_or_default();
        command
            .env("HOME", self.home.path())
            .env("KUBECONFIG", self.home.path().join(".kube/config"))
            .env("PATH", format!("{}:{path}", self.bin.path().display()))
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .env("NO_COLOR", "1");
        command
    }
}

async fn run(command: &mut Command) -> Output {
    let out = command
        .stdin(Stdio::null())
        .output()
        .await
        .expect("command ran");
    Output {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// The `kubectl-proxyauth` built from the tree (`cargo build -p
/// kubectl_proxyauth`, done by `it:test`), or `IT_CLI_BIN`.
fn cli_binary() -> PathBuf {
    if let Some(bin) = std::env::var_os("IT_CLI_BIN") {
        return bin.into();
    }
    let bin = Bench::get()
        .root
        .join("dist/target/debug/kubectl_proxyauth");
    assert!(
        bin.is_file(),
        "{} missing: cargo build -p kubectl_proxyauth",
        bin.display()
    );
    bin
}

/// The authorization URL in one line of the CLI's output, if any.
pub fn authorization_url(line: &str) -> Option<Url> {
    line.split_whitespace()
        .filter(|word| word.starts_with("https://"))
        .find_map(|word| Url::parse(word).ok())
        .filter(|url| url.path().ends_with("/auth"))
}

/// Do what a person does in a browser: open the URL, sign in on Dex's form,
/// and land on the CLI's loopback callback.
async fn play_browser(persona: &Persona, start: &Url, port: u16) {
    let http = Bench::get().http();
    let callback = format!("http://localhost:{port}/");
    let mut url = start.clone();
    for _ in 0..10 {
        if url.as_str().starts_with(&callback) {
            // The CLI's listener: plain HTTP on loopback, like the browser.
            let page = reqwest::get(url.as_str())
                .await
                .expect("CLI callback listener");
            assert!(
                page.status().is_success(),
                "CLI callback answered {}",
                page.status()
            );
            return;
        }
        let response = http
            .get(url.clone())
            .send()
            .await
            .expect("GET in the login flow");
        let response = if response.status() == reqwest::StatusCode::OK {
            let body = response.text().await.expect("login page");
            assert!(
                body.contains("name=\"password\""),
                "unexpected page at {url}: {body}"
            );
            http.post(url.clone())
                .form(&[("login", &persona.email), ("password", &persona.password)])
                .send()
                .await
                .expect("POST Dex login form")
        } else {
            response
        };
        assert!(
            response.status().is_redirection(),
            "{url} answered {} instead of a redirect",
            response.status()
        );
        let location = response.headers()[reqwest::header::LOCATION]
            .to_str()
            .expect("Location header");
        url = url.join(location).expect("redirect URL");
    }
    panic!("the login flow did not reach {callback}");
}

#[cfg(test)]
mod tests {
    use super::authorization_url;

    #[test]
    fn finds_the_authorization_url_in_the_cli_output() {
        let line =
            "INFO Opening your browser to sign in. If nothing happens, open this URL manually:";
        assert_eq!(authorization_url(line), None);
        let line = "https://dex.k8s.localhost:8443/auth?client_id=kube_login&state=x";
        assert_eq!(
            authorization_url(line).map(|u| u.host_str().map(str::to_owned)),
            Some(Some("dex.k8s.localhost".to_string()))
        );
        assert_eq!(authorization_url("see https://example.com/docs"), None);
    }
}
