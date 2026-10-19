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
const CALLBACK_PORTS: std::ops::Range<u16> = 18_000..18_100;

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

    /// The real `oc` (`mise.toml`), with the same kubeconfig as [`Self::kubectl`].
    pub async fn oc(&self, args: &[&str]) -> Output {
        run(self.command(&PathBuf::from("oc")).args(args)).await
    }

    /// `kubectl` stopped after `duration` (a watch never ends by itself): what
    /// it printed until then. `code` is `None` when it had to be killed.
    pub async fn kubectl_for(&self, duration: Duration, args: &[&str]) -> Output {
        let mut child = self
            .command(&PathBuf::from("kubectl"))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn kubectl");
        // Read chunk by chunk into buffers outside the timeout: a cancelled
        // `read_to_string` would lose what it had read.
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let mut out = child.stdout.take().expect("piped stdout");
        let mut err = child.stderr.take().expect("piped stderr");
        let code = tokio::time::timeout(duration, async {
            let (mut out_buf, mut err_buf) = ([0u8; 4096], [0u8; 4096]);
            let (mut out_open, mut err_open) = (true, true);
            while out_open || err_open {
                tokio::select! {
                    read = out.read(&mut out_buf), if out_open => match read {
                        Ok(0) | Err(_) => out_open = false,
                        Ok(n) => stdout.extend_from_slice(&out_buf[..n]),
                    },
                    read = err.read(&mut err_buf), if err_open => match read {
                        Ok(0) | Err(_) => err_open = false,
                        Ok(n) => stderr.extend_from_slice(&err_buf[..n]),
                    },
                }
            }
            child.wait().await.ok().and_then(|s| s.code())
        })
        .await
        .ok()
        .flatten();
        let (stdout, stderr) = (
            String::from_utf8_lossy(&stdout).into_owned(),
            String::from_utf8_lossy(&stderr).into_owned(),
        );
        Output {
            code,
            stdout,
            stderr,
        }
    }

    /// `kubectl port-forward -n <ns> <target> <local>:<remote>`, once kubectl
    /// says it is forwarding; `Err` with what it printed when it exits first.
    pub async fn port_forward(
        &self,
        ns: &str,
        target: &str,
        remote: u16,
    ) -> Result<PortForward, Output> {
        let local = std::net::TcpListener::bind(("127.0.0.1", 0))
            .and_then(|l| l.local_addr())
            .expect("free local port")
            .port();
        let mut child = self
            .command(&PathBuf::from("kubectl"))
            .args([
                "port-forward",
                "-n",
                ns,
                target,
                &format!("{local}:{remote}"),
                "--address",
                "127.0.0.1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn kubectl port-forward");
        let mut lines = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
        let ready = tokio::time::timeout(Duration::from_secs(30), async {
            while let Ok(Some(line)) = lines.next_line().await {
                if line.starts_with("Forwarding from") {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        if ready {
            return Ok(PortForward {
                local,
                _child: child,
                _stdout: lines,
            });
        }
        let _ = child.start_kill();
        let mut stderr = String::new();
        if let Some(mut err) = child.stderr.take() {
            let _ =
                tokio::time::timeout(Duration::from_secs(5), err.read_to_string(&mut stderr)).await;
        }
        let code = child.wait().await.ok().and_then(|s| s.code());
        Err(Output {
            code,
            stdout: String::new(),
            stderr,
        })
    }

    /// The cluster token `get-token` hands kubectl, for raw requests.
    pub async fn cluster_token(&self, ns: &str, cluster: &str) -> String {
        let bench = Bench::get();
        // What kubectl passes an exec plugin.
        let exec_info = serde_json::json!({
            "apiVersion": "client.authentication.k8s.io/v1",
            "kind": "ExecCredential",
            "spec": {
                "interactive": false,
                "cluster": { "server": format!("{}/clusters/{ns}/{cluster}", bench.base_url()) },
            },
        });
        let out = run(self
            .command(&cli_binary())
            .args([
                "get-token",
                &format!("--namespace={ns}"),
                &format!("--server-url={}", bench.base_url()),
                cluster,
            ])
            .env("KUBERNETES_EXEC_INFO", exec_info.to_string()))
        .await;
        assert!(out.success(), "get-token: {out:?}");
        let credential: serde_json::Value =
            serde_json::from_str(&out.stdout).expect("ExecCredential JSON");
        credential["status"]["token"]
            .as_str()
            .expect("status.token")
            .to_string()
    }

    fn command(&self, program: &PathBuf) -> Command {
        let mut command = Command::new(program);
        let path = std::env::var("PATH").unwrap_or_default();
        command
            .env("HOME", self.home.path())
            // Tokens are keyed by server URL in a keyring every test shares:
            // without a service of its own, a `nogroup` login running alongside
            // would replace this persona's server token.
            .env(
                "PROXYAUTH_KEYRING_SERVICE",
                format!("proxyauthk8s-it-{}", self.home.path().display()),
            )
            .env("KUBECONFIG", self.home.path().join(".kube/config"))
            .env("PATH", format!("{}:{path}", self.bin.path().display()))
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .env("NO_COLOR", "1");
        command
    }
}

/// A running `kubectl port-forward`, stopped on drop.
pub struct PortForward {
    /// The local port it listens on (127.0.0.1).
    pub local: u16,
    _child: tokio::process::Child,
    // Kept open so kubectl never blocks writing to a closed pipe.
    _stdout: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}

impl PortForward {
    /// Wait up to `timeout` for kubectl to exit on its own (a refused stream
    /// ends the session), and return what it printed on stderr. `code` is
    /// `None` when it was still running and had to be killed.
    pub async fn exit(mut self, timeout: Duration) -> Output {
        let code = tokio::time::timeout(timeout, self._child.wait())
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|s| s.code());
        if code.is_none() {
            let _ = self._child.kill().await;
        }
        let mut stderr = String::new();
        if let Some(mut err) = self._child.stderr.take() {
            let _ =
                tokio::time::timeout(Duration::from_secs(5), err.read_to_string(&mut stderr)).await;
        }
        Output {
            code,
            stdout: String::new(),
            stderr,
        }
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
