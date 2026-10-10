//! Trust on first use for a server whose TLS certificate the system does not
//! trust (`oc login`-style).
//!
//! Before the first login to an `https` server with no CA configured, the
//! server's chain is fetched with a handshake that is aborted as soon as the
//! certificate arrives (nothing is ever sent to an unverified server). When the
//! system refuses the chain only because its issuer is unknown, and the chain
//! ends with a self-signed certificate that does verify the server for its
//! hostname, the user is shown that certificate's fingerprint and asked whether
//! to trust it. Accepting saves it exactly like `--certificate-authority`.
//!
//! Anything else (wrong hostname, expired, a chain that does not carry its own
//! root) is never offered: pinning would either fail or hide a real problem.

use std::{
    fmt::Write as _,
    io::{self, BufRead, IsTerminal, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use reqwest::Url;
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore,
    SignatureScheme,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject as _},
};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tracing::{debug, warn};
use x509_cert::{Certificate, der::Decode};

use crate::error::ProxyAuthK8sError;

/// Upper bound on each of the TCP connect and the TLS handshake of the probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// The certificate the user is asked to trust, as shown to them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PresentedCertificate {
    pub der: CertificateDer<'static>,
    pub subject: String,
    pub not_before: String,
    pub not_after: String,
    /// SHA-256 of the DER, colon-separated upper-case hex (as `openssl x509
    /// -fingerprint -sha256` prints it).
    pub sha256: String,
}

impl PresentedCertificate {
    fn parse(der: &CertificateDer<'static>) -> Result<(Self, bool), String> {
        let cert = Certificate::from_der(der.as_ref()).map_err(|e| e.to_string())?;
        let tbs = cert.tbs_certificate();
        let self_signed = tbs.subject() == tbs.issuer();
        let validity = tbs.validity();
        Ok((
            Self {
                der: der.clone(),
                subject: tbs.subject().to_string(),
                not_before: validity.not_before.to_string(),
                not_after: validity.not_after.to_string(),
                sha256: fingerprint(der.as_ref()),
            },
            self_signed,
        ))
    }

    /// Base64 PEM, the form `certificate_authority_data` stores.
    fn certificate_authority_data(&self) -> String {
        let body = BASE64.encode(self.der.as_ref());
        let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
        for line in body.as_bytes().chunks(64) {
            pem.push_str(&String::from_utf8_lossy(line));
            pem.push('\n');
        }
        pem.push_str("-----END CERTIFICATE-----\n");
        BASE64.encode(pem)
    }
}

fn fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    let mut out = String::with_capacity(digest.len() * 3);
    for (i, byte) in digest.iter().enumerate() {
        if i > 0 {
            out.push(':');
        }
        let _ = write!(out, "{byte:02X}");
    }
    out
}

/// What to do with the chain a server presented.
#[derive(Debug)]
pub(super) enum Assessment {
    /// The system trusts it: nothing to ask.
    Trusted,
    /// Unknown to the system, but a self-signed certificate that verifies the
    /// server: it can be pinned.
    Pinnable(PresentedCertificate),
    /// Not trusted, and pinning would not be safe or would not work.
    Untrusted(String),
}

/// Decide whether `chain` (leaf first) can be offered for trust on first use.
pub(super) fn assess(
    chain: &[CertificateDer<'static>],
    server_name: &ServerName<'static>,
    system: &dyn ServerCertVerifier,
    provider: &Arc<CryptoProvider>,
    now: UnixTime,
) -> Assessment {
    let Some((leaf, intermediates)) = chain.split_first() else {
        return Assessment::Untrusted("the server presented no certificate".to_string());
    };
    match system.verify_server_cert(leaf, intermediates, server_name, &[], now) {
        Ok(_) => return Assessment::Trusted,
        Err(TlsError::InvalidCertificate(CertificateError::UnknownIssuer)) => {}
        Err(e) => return Assessment::Untrusted(e.to_string()),
    }

    // Only a self-signed certificate is pinned: a root or a self-signed leaf
    // is accepted as a trust anchor by every TLS stack (kubectl included),
    // an intermediate is not.
    let top = chain.last().unwrap_or(leaf);
    let (presented, self_signed) = match PresentedCertificate::parse(top) {
        Ok(parsed) => parsed,
        Err(e) => return Assessment::Untrusted(format!("unreadable certificate: {e}")),
    };
    if !self_signed {
        return Assessment::Untrusted(
            "the server does not send the root of its certificate chain".to_string(),
        );
    }

    let mut roots = RootCertStore::empty();
    if let Err(e) = roots.add(top.clone()) {
        return Assessment::Untrusted(format!("unusable certificate: {e}"));
    }
    let pinned = match WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        provider.clone(),
    )
    .build()
    {
        Ok(verifier) => verifier,
        Err(e) => return Assessment::Untrusted(e.to_string()),
    };
    // Trusting it must actually fix the connection: same hostname and
    // validity checks as any later request.
    match pinned.verify_server_cert(leaf, intermediates, server_name, &[], now) {
        Ok(_) => Assessment::Pinnable(presented),
        Err(e) => Assessment::Untrusted(e.to_string()),
    }
}

/// Records the chain the server presents, then aborts the handshake.
#[derive(Debug)]
struct CaptureChain {
    chain: Mutex<Option<Vec<CertificateDer<'static>>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for CaptureChain {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let chain = std::iter::once(end_entity)
            .chain(intermediates)
            .map(|cert| cert.clone().into_owned())
            .collect();
        if let Ok(mut slot) = self.chain.lock() {
            *slot = Some(chain);
        }
        // Never complete a handshake with a server that was not verified.
        Err(TlsError::General(
            "certificate captured for inspection".to_string(),
        ))
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("probe only".to_string()))
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Err(TlsError::General("probe only".to_string()))
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// `(host, port, server name)` of an `https` URL; `None` for anything else.
fn https_target(url: &Url) -> Option<(String, u16, ServerName<'static>)> {
    if url.scheme() != "https" {
        return None;
    }
    let port = url.port_or_known_default()?;
    let (host, server_name) = match url.host()? {
        url::Host::Domain(domain) => (
            domain.to_string(),
            ServerName::try_from(domain.to_string()).ok()?,
        ),
        url::Host::Ipv4(ip) => (ip.to_string(), ServerName::IpAddress(ip.into())),
        url::Host::Ipv6(ip) => (ip.to_string(), ServerName::IpAddress(ip.into())),
    };
    Some((host, port, server_name))
}

/// Fetch the certificate chain `host:port` presents for `server_name`.
pub(super) async fn probe_chain(
    host: &str,
    port: u16,
    server_name: ServerName<'static>,
) -> Result<Vec<CertificateDer<'static>>, String> {
    let provider = provider();
    let capture = Arc::new(CaptureChain {
        chain: Mutex::new(None),
        provider: provider.clone(),
    });
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(capture.clone())
        .with_no_client_auth();

    let tcp = tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| "timed out connecting".to_string())?
        .map_err(|e| e.to_string())?;
    // Expected to fail: the verifier aborts once it has the chain.
    let _ = tokio::time::timeout(
        PROBE_TIMEOUT,
        TlsConnector::from(Arc::new(config)).connect(server_name, tcp),
    )
    .await;

    capture
        .chain
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .ok_or_else(|| "the server presented no certificate".to_string())
}

/// What a server login has to decide about the server's certificate.
#[derive(Debug)]
pub(super) enum Inspection {
    /// Nothing to ask: plain http, unreachable, or the saved CA (or the
    /// system) still verifies the server.
    Keep,
    /// No CA saved yet for this server.
    FirstUse(Assessment),
    /// A CA was saved, and it no longer verifies the server: rotated
    /// certificate, or someone in the middle.
    Changed {
        /// SHA-256 of each certificate saved for the server.
        previous: Vec<String>,
        assessment: Assessment,
    },
}

/// The certificates in `certificate_authority_data` (base64 PEM), if readable.
fn saved_certificates(certificate_authority_data: &str) -> Option<Vec<CertificateDer<'static>>> {
    let pem = BASE64.decode(certificate_authority_data).ok()?;
    let certs = CertificateDer::pem_slice_iter(&pem)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (!certs.is_empty()).then_some(certs)
}

/// Whether `chain` verifies for `server_name` against `anchors` alone.
fn verifies_against(
    anchors: &[CertificateDer<'static>],
    chain: &[CertificateDer<'static>],
    server_name: &ServerName<'static>,
    provider: &Arc<CryptoProvider>,
    now: UnixTime,
) -> bool {
    let Some((leaf, intermediates)) = chain.split_first() else {
        return false;
    };
    let mut roots = RootCertStore::empty();
    let (added, _) = roots.add_parsable_certificates(anchors.iter().cloned());
    if added == 0 {
        return false;
    }
    WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .is_ok_and(|verifier| {
            verifier
                .verify_server_cert(leaf, intermediates, server_name, &[], now)
                .is_ok()
        })
}

/// Compare what `server_url` presents with what is saved for it.
pub(super) async fn inspect(
    server_url: &str,
    saved: Option<&str>,
    system: &dyn ServerCertVerifier,
    now: UnixTime,
) -> Inspection {
    let Some((host, port, server_name)) =
        Url::parse(server_url).ok().as_ref().and_then(https_target)
    else {
        return Inspection::Keep;
    };
    let chain = match probe_chain(&host, port, server_name.clone()).await {
        Ok(chain) => chain,
        Err(e) => {
            // The login request that follows reports the real failure.
            debug!(error = %e, "could not fetch the server certificate");
            return Inspection::Keep;
        }
    };
    let provider = provider();
    let Some(saved) = saved else {
        return Inspection::FirstUse(assess(&chain, &server_name, system, &provider, now));
    };
    let Some(anchors) = saved_certificates(saved) else {
        // A corrupted entry is reported by the HTTP client that loads it.
        return Inspection::Keep;
    };
    let still_trusted = verifies_against(&anchors, &chain, &server_name, &provider, now)
        || chain.split_first().is_some_and(|(leaf, intermediates)| {
            system
                .verify_server_cert(leaf, intermediates, &server_name, &[], now)
                .is_ok()
        });
    if still_trusted {
        return Inspection::Keep;
    }
    Inspection::Changed {
        previous: anchors
            .iter()
            .map(|cert| fingerprint(cert.as_ref()))
            .collect(),
        assessment: assess(&chain, &server_name, system, &provider, now),
    }
}

/// Turn an inspection into the CA to save from now on, asking the user when
/// needed. `None` keeps whatever is saved.
///
/// `interactive` is false when stdin is not a terminal: nothing is read, and
/// an untrusted server is left to fail on the request that follows.
pub(super) fn decide(
    server_url: &str,
    inspection: Inspection,
    interactive: bool,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<Option<String>, ProxyAuthK8sError> {
    let (previous, assessment) = match inspection {
        Inspection::Keep => return Ok(None),
        Inspection::FirstUse(assessment) => (None, assessment),
        Inspection::Changed {
            previous,
            assessment,
        } => (Some(previous), assessment),
    };
    let presented = match assessment {
        Assessment::Trusted => return Ok(None),
        Assessment::Untrusted(reason) => {
            match previous {
                Some(_) => warn!(
                    "The certificate of {server_url} no longer matches the one trusted for it, \
                     and the new one cannot be trusted automatically ({reason}). Get the CA \
                     from the server's administrator and pass it with --certificate-authority."
                ),
                None => warn!(
                    "The certificate of {server_url} is not trusted ({reason}). Get the CA \
                     from the server's administrator and pass it with --certificate-authority."
                ),
            }
            return Ok(None);
        }
        Assessment::Pinnable(presented) => presented,
    };

    if !interactive {
        warn!(
            sha256 = %presented.sha256,
            "The certificate of {server_url} is self-signed and {}. Run the login from a \
             terminal to review it, or pass it with --certificate-authority.",
            if previous.is_some() {
                "has changed since it was trusted"
            } else {
                "not trusted by this system"
            }
        );
        return Ok(None);
    }

    let details = format!(
        "\x20 Subject:  {}\n\
         \x20 Valid:    {} to {}\n\
         \x20 SHA-256:  {}\n",
        presented.subject, presented.not_before, presented.not_after, presented.sha256,
    );
    let prompt = match &previous {
        None => format!(
            "The server {server_url} presents a self-signed certificate this system does not trust:\n\
             {details}\
             Check this fingerprint with the server's administrator before accepting.\n\
             Trust this certificate for {server_url}? [y/N]: "
        ),
        Some(previous) => format!(
            "WARNING: the certificate of {server_url} has CHANGED since it was trusted.\n\
             Previously trusted SHA-256: {}\n\
             Now presented:\n\
             {details}\
             This is expected after a certificate renewal, but it is also what an attacker\n\
             intercepting the connection would show. Check the new fingerprint with the\n\
             server's administrator before accepting.\n\
             Replace the trusted certificate for {server_url}? [y/N]: ",
            previous.join(", "),
        ),
    };
    if confirm(&prompt, input, output) {
        Ok(Some(presented.certificate_authority_data()))
    } else {
        let what = if previous.is_some() {
            "changed and the new certificate"
        } else {
            "presents a certificate that"
        };
        Err(ProxyAuthK8sError::UntrustedServerCertificate(format!(
            "{server_url} {what} (SHA-256 {}) was not trusted",
            presented.sha256
        )))
    }
}

/// Print `prompt` and read a yes/no answer; anything but `y`/`yes` is no.
fn confirm(prompt: &str, input: &mut dyn BufRead, output: &mut dyn Write) -> bool {
    if write!(output, "{prompt}").is_err() || output.flush().is_err() {
        return false;
    }
    let mut answer = String::new();
    if input.read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// The CA to save for `server_url` from now on, when it changes: a
/// certificate trusted on first use, or a rotated one the user accepted.
/// `None` keeps `saved` as it is.
pub(super) async fn check_server_certificate(
    server_url: &str,
    saved: Option<&str>,
) -> Result<Option<String>, ProxyAuthK8sError> {
    let system = match rustls_platform_verifier::Verifier::new(provider()) {
        Ok(verifier) => verifier,
        Err(e) => {
            debug!(error = %e, "system certificate verifier unavailable");
            return Ok(None);
        }
    };
    let inspection = inspect(server_url, saved, &system, UnixTime::now()).await;
    let stdin = io::stdin();
    // Tests never prompt, whatever runs them.
    let interactive = !cfg!(test) && stdin.is_terminal();
    decide(
        server_url,
        inspection,
        interactive,
        &mut stdin.lock(),
        &mut io::stdout(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_config::cli_server_config::http_client;
    use crate::test_support::{
        TlsTestServer, ca_cert, certificate_authority_data, leaf_cert, self_signed_cert,
    };
    use std::io::Cursor;

    fn name(host: &str) -> ServerName<'static> {
        ServerName::try_from(host.to_string()).expect("server name")
    }

    /// A "system" that trusts only `anchor`.
    fn system_trusting(anchor: &CertificateDer<'static>) -> Arc<WebPkiServerVerifier> {
        let mut roots = RootCertStore::empty();
        roots.add(anchor.clone()).expect("anchor");
        WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider())
            .build()
            .expect("verifier")
    }

    /// A "system" that trusts nothing the tests generate.
    fn unrelated_system() -> Arc<WebPkiServerVerifier> {
        system_trusting(&ca_cert("some public root").cert)
    }

    fn assess_now(
        chain: &[CertificateDer<'static>],
        host: &str,
        system: &dyn ServerCertVerifier,
    ) -> Assessment {
        assess(chain, &name(host), system, &provider(), UnixTime::now())
    }

    // --- assess ---------------------------------------------------------

    #[test]
    fn a_self_signed_certificate_for_the_host_can_be_pinned() {
        let server = self_signed_cert("proxy", &["proxy.example"]);
        let assessment = assess_now(
            std::slice::from_ref(&server.cert),
            "proxy.example",
            unrelated_system().as_ref(),
        );
        let Assessment::Pinnable(presented) = assessment else {
            panic!("expected a pinnable certificate, got {assessment:?}");
        };
        assert_eq!(presented.der, server.cert);
        assert_eq!(presented.subject, "CN=proxy");
        assert_eq!(presented.sha256, fingerprint(server.cert.as_ref()));
        assert_eq!(presented.sha256.len(), 32 * 3 - 1);
    }

    #[test]
    fn a_chain_the_system_trusts_is_not_asked_about() {
        let root = ca_cert("internal root");
        let leaf = leaf_cert(&root, "proxy", &["proxy.example"]);
        let assessment = assess_now(
            &[leaf.cert],
            "proxy.example",
            system_trusting(&root.cert).as_ref(),
        );
        assert!(matches!(assessment, Assessment::Trusted), "{assessment:?}");
    }

    #[test]
    fn the_root_is_pinned_when_the_server_sends_it() {
        let root = ca_cert("internal root");
        let leaf = leaf_cert(&root, "proxy", &["proxy.example"]);
        let assessment = assess_now(
            &[leaf.cert, root.cert.clone()],
            "proxy.example",
            unrelated_system().as_ref(),
        );
        let Assessment::Pinnable(presented) = assessment else {
            panic!("expected the root to be pinnable, got {assessment:?}");
        };
        assert_eq!(presented.der, root.cert);
        assert_eq!(presented.subject, "CN=internal root");
    }

    #[test]
    fn a_chain_without_its_root_is_not_offered() {
        let root = ca_cert("internal root");
        let leaf = leaf_cert(&root, "proxy", &["proxy.example"]);
        let assessment = assess_now(&[leaf.cert], "proxy.example", unrelated_system().as_ref());
        let Assessment::Untrusted(reason) = assessment else {
            panic!("an intermediate-anchored chain must not be pinned, got {assessment:?}");
        };
        assert!(reason.contains("root"), "{reason}");
    }

    #[test]
    fn a_certificate_for_another_host_is_not_offered() {
        let server = self_signed_cert("proxy", &["other.example"]);
        let assessment = assess_now(&[server.cert], "proxy.example", unrelated_system().as_ref());
        assert!(
            matches!(assessment, Assessment::Untrusted(_)),
            "{assessment:?}"
        );
    }

    #[test]
    fn an_expired_certificate_is_not_offered() {
        let server = self_signed_cert("proxy", &["proxy.example"]);
        // rcgen certificates are valid until 4096; check them in 5000.
        let far_future = UnixTime::since_unix_epoch(Duration::from_secs(95_617_584_000));
        let assessment = assess(
            &[server.cert],
            &name("proxy.example"),
            unrelated_system().as_ref(),
            &provider(),
            far_future,
        );
        assert!(
            matches!(assessment, Assessment::Untrusted(_)),
            "{assessment:?}"
        );
    }

    #[test]
    fn an_empty_chain_is_not_offered() {
        let assessment = assess_now(&[], "proxy.example", unrelated_system().as_ref());
        assert!(
            matches!(assessment, Assessment::Untrusted(_)),
            "{assessment:?}"
        );
    }

    // --- decide ---------------------------------------------------------

    fn pinnable() -> PresentedCertificate {
        let server = self_signed_cert("proxy", &["proxy.example"]);
        PresentedCertificate::parse(&server.cert).expect("parse").0
    }

    fn first_use(presented: PresentedCertificate) -> Inspection {
        Inspection::FirstUse(Assessment::Pinnable(presented))
    }

    fn changed(presented: PresentedCertificate) -> Inspection {
        Inspection::Changed {
            previous: vec!["AA:BB".to_string()],
            assessment: Assessment::Pinnable(presented),
        }
    }

    fn decide_with(
        inspection: Inspection,
        answer: &str,
    ) -> (Result<Option<String>, ProxyAuthK8sError>, String) {
        let mut output = Vec::new();
        let result = decide(
            "https://proxy.example",
            inspection,
            true,
            &mut Cursor::new(answer.to_string()),
            &mut output,
        );
        (result, String::from_utf8(output).unwrap())
    }

    #[test]
    fn accepting_returns_the_certificate_as_certificate_authority_data() {
        let presented = pinnable();
        let (result, shown) = decide_with(first_use(presented.clone()), "y\n");
        let data = result.expect("accepted").expect("a CA to save");

        assert!(shown.contains(&presented.sha256), "{shown}");
        assert!(shown.contains("CN=proxy"), "{shown}");
        assert!(!shown.contains("CHANGED"), "{shown}");
        // Same encoding as `--certificate-authority`, usable by the client.
        http_client(Some(&data)).expect("the pinned certificate builds a client");
        let pem = String::from_utf8(BASE64.decode(&data).unwrap()).unwrap();
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----\n"), "{pem}");
    }

    #[test]
    fn a_changed_certificate_is_shown_next_to_the_trusted_one() {
        let presented = pinnable();
        let (result, shown) = decide_with(changed(presented.clone()), "yes\n");
        assert!(result.expect("accepted").is_some());
        assert!(shown.contains("CHANGED"), "{shown}");
        assert!(shown.contains("AA:BB"), "{shown}");
        assert!(shown.contains(&presented.sha256), "{shown}");
        assert!(shown.contains("Replace the trusted certificate"), "{shown}");

        let (result, _) = decide_with(changed(pinnable()), "\n");
        let err = result.expect_err("the default is to keep refusing");
        assert!(err.to_string().contains("changed"), "{err}");
    }

    #[test]
    fn anything_but_yes_refuses() {
        for answer in ["\n", "n\n", "no\n", "yess\n", ""] {
            let (result, _) = decide_with(first_use(pinnable()), answer);
            let err = result.expect_err("not accepted");
            assert!(
                matches!(err, ProxyAuthK8sError::UntrustedServerCertificate(_)),
                "{answer:?}: {err:?}"
            );
        }
        for answer in ["Y\n", " yes \n", "YES\n"] {
            let (result, _) = decide_with(first_use(pinnable()), answer);
            assert!(result.expect("accepted").is_some(), "{answer:?}");
        }
    }

    #[test]
    fn nothing_is_asked_without_a_terminal_or_a_pinnable_certificate() {
        let cases = [
            (first_use(pinnable()), false),
            (changed(pinnable()), false),
            (Inspection::Keep, true),
            (Inspection::FirstUse(Assessment::Trusted), true),
            (
                Inspection::FirstUse(Assessment::Untrusted("expired".to_string())),
                true,
            ),
            (
                Inspection::Changed {
                    previous: vec!["AA:BB".to_string()],
                    assessment: Assessment::Untrusted("wrong host".to_string()),
                },
                true,
            ),
        ];
        for (inspection, interactive) in cases {
            let mut output = Vec::new();
            let saved = decide(
                "https://proxy.example",
                inspection,
                interactive,
                // Would accept if it were read.
                &mut Cursor::new("y\n"),
                &mut output,
            )
            .expect("no error");
            assert_eq!(saved, None);
            assert!(output.is_empty(), "nothing must be prompted");
        }
    }

    #[test]
    fn only_https_urls_are_probed() {
        assert!(https_target(&Url::parse("http://proxy.example").unwrap()).is_none());
        let (host, port, _) = https_target(&Url::parse("https://proxy.example").unwrap()).unwrap();
        assert_eq!((host.as_str(), port), ("proxy.example", 443));
        let (host, port, name) =
            https_target(&Url::parse("https://127.0.0.1:8443/base").unwrap()).unwrap();
        assert_eq!((host.as_str(), port), ("127.0.0.1", 8443));
        assert!(matches!(name, ServerName::IpAddress(_)));
        let (host, _, _) = https_target(&Url::parse("https://[::1]:8443").unwrap()).unwrap();
        assert_eq!(host, "::1");
    }

    // --- against a real TLS server --------------------------------------

    #[tokio::test]
    async fn the_probe_fetches_the_chain_and_the_pin_then_works() {
        let cert = self_signed_cert("proxy", &["localhost"]);
        let server = TlsTestServer::start(&[&cert]).await;

        let chain = probe_chain("127.0.0.1", server.port, name("localhost"))
            .await
            .expect("chain");
        assert_eq!(chain, vec![cert.cert.clone()]);

        let inspection = inspect(
            &server.url(),
            None,
            unrelated_system().as_ref(),
            UnixTime::now(),
        )
        .await;
        let (result, _) = decide_with(inspection, "yes\n");
        let data = result.unwrap().expect("pinned");
        assert_eq!(data, certificate_authority_data(&cert));

        // The saved CA is what every later request trusts, through the
        // CLI's real HTTP client and its default TLS backend.
        let url = format!("{}/api/v1/clusters", server.url());
        assert!(http_client(None).unwrap().get(&url).send().await.is_err());
        let response = http_client(Some(&data))
            .unwrap()
            .get(&url)
            .send()
            .await
            .expect("the pinned certificate is trusted");
        assert!(response.status().is_success());
    }

    #[tokio::test]
    async fn a_rotated_certificate_is_detected_and_a_kept_one_is_not() {
        let before = self_signed_cert("proxy", &["localhost"]);
        let server = TlsTestServer::start(&[&before]).await;
        let saved = certificate_authority_data(&before);
        let system = unrelated_system();

        let inspection = inspect(
            &server.url(),
            Some(&saved),
            system.as_ref(),
            UnixTime::now(),
        )
        .await;
        assert!(matches!(inspection, Inspection::Keep), "{inspection:?}");

        // Renewal with a new key: the saved pin no longer verifies.
        let after = self_signed_cert("proxy", &["localhost"]);
        server.rotate(&[&after]);
        let inspection = inspect(
            &server.url(),
            Some(&saved),
            system.as_ref(),
            UnixTime::now(),
        )
        .await;
        let Inspection::Changed {
            previous,
            assessment: Assessment::Pinnable(presented),
        } = inspection
        else {
            panic!("expected a pinnable change, got {inspection:?}");
        };
        assert_eq!(previous, vec![fingerprint(before.cert.as_ref())]);
        assert_eq!(presented.der, after.cert);
    }

    #[tokio::test]
    async fn a_pinned_root_survives_a_leaf_rotation() {
        let root = ca_cert("internal root");
        let server =
            TlsTestServer::start(&[&leaf_cert(&root, "proxy", &["localhost"]), &root]).await;
        let saved = certificate_authority_data(&root);
        server.rotate(&[&leaf_cert(&root, "proxy", &["localhost"]), &root]);
        let inspection = inspect(
            &server.url(),
            Some(&saved),
            unrelated_system().as_ref(),
            UnixTime::now(),
        )
        .await;
        assert!(matches!(inspection, Inspection::Keep), "{inspection:?}");
    }

    #[tokio::test]
    async fn a_server_moving_to_a_trusted_certificate_keeps_working() {
        let old = self_signed_cert("proxy", &["localhost"]);
        let root = ca_cert("public root");
        let server = TlsTestServer::start(&[&leaf_cert(&root, "proxy", &["localhost"])]).await;
        // The saved pin no longer matches, but the system trusts the new
        // chain and the client merges both: nothing to ask.
        let inspection = inspect(
            &server.url(),
            Some(&certificate_authority_data(&old)),
            system_trusting(&root.cert).as_ref(),
            UnixTime::now(),
        )
        .await;
        assert!(matches!(inspection, Inspection::Keep), "{inspection:?}");
    }

    #[tokio::test]
    async fn an_unreachable_server_is_left_to_the_login_request() {
        // Port 9 (discard) is closed on loopback.
        assert!(
            probe_chain("127.0.0.1", 9, name("localhost"))
                .await
                .is_err()
        );
        for (url, saved) in [
            ("https://127.0.0.1:9", None),
            ("https://127.0.0.1:9", Some("c2F2ZWQ=")),
            ("http://127.0.0.1:9", None),
        ] {
            assert_eq!(check_server_certificate(url, saved).await.unwrap(), None);
        }
    }
}
