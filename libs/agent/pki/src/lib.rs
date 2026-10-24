//! The agent CA of RFC 0012 (§5.2, decision 9).
//!
//! One CA per proxy installation, distinct from any cluster CA, signs two
//! kinds of certificates: an agent's client certificate, whose only URI SAN is
//! its [`AgentIdentity`], and each proxy replica's server certificate for the
//! mTLS inside the WebSocket ([`TUNNEL_SERVER_NAME`]). The CA never signs what
//! a request asks for: of a CSR, only the public key is kept.

use std::time::Duration;

use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, DistinguishedName,
    DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256, SanType, SerialNumber,
};
use time::OffsetDateTime;
use x509_parser::extensions::GeneralName;
use x509_parser::prelude::{FromDer, X509Certificate};

/// Secret of the proxy namespace holding the CA (`tls.crt`, `tls.key`).
pub const CA_SECRET: &str = "proxy-auth-k8s-agent-ca";
/// SAN of every replica's tunnel server certificate; agents check it.
pub const TUNNEL_SERVER_NAME: &str = "agent-tunnel.proxyauth.internal";
/// Lifetime of an agent certificate; renewed at two thirds.
pub const AGENT_CERT_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
/// Lifetime of a replica's tunnel server certificate.
pub const SERVER_CERT_LIFETIME: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const CA_LIFETIME: Duration = Duration::from_secs(10 * 365 * 24 * 60 * 60);
/// Tolerance for a peer whose clock runs a little behind.
const BACKDATE: Duration = Duration::from_secs(5 * 60);
const URI_SCHEME: &str = "proxyauth://";

#[derive(Debug, thiserror::Error)]
pub enum PkiError {
    #[error("certificate generation: {0}")]
    Rcgen(#[from] rcgen::Error),
    #[error("certificate parsing: {0}")]
    Parse(String),
    #[error("the certificate carries no agent identity")]
    NoIdentity,
    #[error("the certificate carries more than one agent identity")]
    SeveralIdentities,
    #[error("invalid agent identity {0:?}")]
    InvalidIdentity(String),
    #[error("validity out of range")]
    Validity,
    #[error("no randomness for a serial number")]
    Random,
}

/// Who an agent certificate speaks for: one `ProxyKubeApi`, by namespace, name
/// and UID (a CR recreated under the same name inherits no agent), at the
/// `epoch` it was issued for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AgentIdentity {
    pub namespace: String,
    pub name: String,
    pub uid: String,
    pub epoch: u64,
}

fn is_name(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 253
        && segment
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

fn is_uid(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 64
        && segment.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

impl AgentIdentity {
    /// `proxyauth://<namespace>/<name>/<uid>/<epoch>`.
    #[must_use]
    pub fn to_uri(&self) -> String {
        format!(
            "{URI_SCHEME}{}/{}/{}/{}",
            self.namespace, self.name, self.uid, self.epoch
        )
    }

    pub fn from_uri(uri: &str) -> Result<Self, PkiError> {
        let invalid = || PkiError::InvalidIdentity(uri.to_string());
        let rest = uri.strip_prefix(URI_SCHEME).ok_or_else(invalid)?;
        let mut parts = rest.split('/');
        let (Some(namespace), Some(name), Some(uid), Some(epoch), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return Err(invalid());
        };
        if !is_name(namespace) || !is_name(name) || !is_uid(uid) {
            return Err(invalid());
        }
        if epoch.is_empty() || !epoch.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        Ok(Self {
            namespace: namespace.to_string(),
            name: name.to_string(),
            uid: uid.to_string(),
            epoch: epoch.parse().map_err(|_| invalid())?,
        })
    }

    /// `<namespace>/<name>`, the cluster as the proxy routes it.
    #[must_use]
    pub fn cluster(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

/// What a peer certificate proves, once the TLS layer has checked its chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerCertificate {
    pub identity: AgentIdentity,
    pub not_after: OffsetDateTime,
}

/// Read the identity of a certificate the agent CA issued.
pub fn identity_of(cert_der: &[u8]) -> Result<PeerCertificate, PkiError> {
    let (_, cert) =
        X509Certificate::from_der(cert_der).map_err(|err| PkiError::Parse(err.to_string()))?;
    let mut uris = Vec::new();
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for name in &san.value.general_names {
            if let GeneralName::URI(uri) = name
                && uri.starts_with(URI_SCHEME)
            {
                uris.push(*uri);
            }
        }
    }
    let uri = match uris.as_slice() {
        [] => return Err(PkiError::NoIdentity),
        [uri] => *uri,
        _ => return Err(PkiError::SeveralIdentities),
    };
    Ok(PeerCertificate {
        identity: AgentIdentity::from_uri(uri)?,
        not_after: cert.validity().not_after.to_datetime(),
    })
}

fn validity(params: &mut CertificateParams, lifetime: Duration) -> Result<(), PkiError> {
    let now = OffsetDateTime::now_utc();
    params.not_before = now
        .checked_sub(BACKDATE.try_into().map_err(|_| PkiError::Validity)?)
        .ok_or(PkiError::Validity)?;
    params.not_after = now
        .checked_add(lifetime.try_into().map_err(|_| PkiError::Validity)?)
        .ok_or(PkiError::Validity)?;
    Ok(())
}

fn random_serial() -> Result<SerialNumber, PkiError> {
    // rcgen's default serial is a hash of the public key: an agent renewing
    // with the same key would get the same serial twice.
    let mut serial = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut serial)
        .map_err(|_| PkiError::Random)?;
    // Positive and never zero-led, so it encodes on exactly 16 bytes.
    serial[0] = (serial[0] & 0x7f) | 0x40;
    Ok(SerialNumber::from_slice(&serial))
}

/// An agent's own key: ECDSA P-256, held in memory only
/// (`ServiceAccountToken`, §6.5), and the CSRs it signs.
pub struct AgentKey(KeyPair);

impl AgentKey {
    pub fn generate() -> Result<Self, PkiError> {
        Ok(Self(KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?))
    }

    /// For the TLS client: never logged.
    #[must_use]
    pub fn key_pem(&self) -> String {
        self.0.serialize_pem()
    }

    /// A CSR for this key. Its subject is ignored: the gateway names the
    /// identity ([`AgentCa::issue_agent`]).
    pub fn csr_pem(&self) -> Result<String, PkiError> {
        Ok(CertificateParams::default()
            .serialize_request(&self.0)?
            .pem()?)
    }
}

impl std::fmt::Debug for AgentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentKey").finish_non_exhaustive()
    }
}

/// The agent CA, loaded from or written to [`CA_SECRET`].
pub struct AgentCa {
    cert_pem: String,
    key_pem: String,
    issuer: Issuer<'static, KeyPair>,
}

impl AgentCa {
    /// A new CA: ECDSA P-256, ten years.
    pub fn generate() -> Result<Self, PkiError> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "proxyauthk8s agent CA");
        params.distinguished_name = dn;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.serial_number = Some(random_serial()?);
        validity(&mut params, CA_LIFETIME)?;
        let cert = params.self_signed(&key)?;
        let key_pem = key.serialize_pem();
        Ok(Self {
            cert_pem: cert.pem(),
            issuer: Issuer::new(params, key),
            key_pem,
        })
    }

    pub fn from_pem(cert_pem: &str, key_pem: &str) -> Result<Self, PkiError> {
        let key = KeyPair::from_pem(key_pem)?;
        Ok(Self {
            issuer: Issuer::from_ca_cert_pem(cert_pem, key)?,
            cert_pem: cert_pem.to_string(),
            key_pem: key_pem.to_string(),
        })
    }

    #[must_use]
    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    /// For the Secret only: never logged (T12).
    #[must_use]
    pub fn key_pem(&self) -> &str {
        &self.key_pem
    }

    /// Sign an agent certificate for `identity` with the public key of
    /// `csr_pem`. The CSR's signature must verify (proof the agent holds the
    /// key); its subject and extensions are ignored.
    pub fn issue_agent(&self, csr_pem: &str, identity: &AgentIdentity) -> Result<String, PkiError> {
        let csr = CertificateSigningRequestParams::from_pem(csr_pem)?;
        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, identity.cluster());
        params.distinguished_name = dn;
        params.subject_alt_names = vec![SanType::URI(identity.to_uri().try_into()?)];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        params.serial_number = Some(random_serial()?);
        validity(&mut params, AGENT_CERT_LIFETIME)?;
        Ok(params.signed_by(&csr.public_key, &self.issuer)?.pem())
    }

    /// A replica's tunnel server certificate and its new key, PEM.
    pub fn issue_server(&self) -> Result<(String, String), PkiError> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let mut params = CertificateParams::new(vec![TUNNEL_SERVER_NAME.to_string()])?;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.serial_number = Some(random_serial()?);
        validity(&mut params, SERVER_CERT_LIFETIME)?;
        Ok((
            params.signed_by(&key, &self.issuer)?.pem(),
            key.serialize_pem(),
        ))
    }
}

impl std::fmt::Debug for AgentCa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // T12: the key never reaches a log.
        f.debug_struct("AgentCa").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_key_s_csr_is_signed_for_the_gateway_s_identity() {
        let ca = AgentCa::generate().unwrap();
        let key = AgentKey::generate().unwrap();
        let identity = AgentIdentity {
            namespace: "team-a".into(),
            name: "prod".into(),
            uid: "aa".into(),
            epoch: 1,
        };
        let cert = ca.issue_agent(&key.csr_pem().unwrap(), &identity).unwrap();
        let der = rcgen_der(&cert);
        assert_eq!(identity_of(&der).unwrap().identity, identity);
    }

    fn identity() -> AgentIdentity {
        AgentIdentity {
            namespace: "team-a".into(),
            name: "prod".into(),
            uid: "6c1f3a52-0b8e-4b7f-9d2a-3e4f5a6b7c8d".into(),
            epoch: 3,
        }
    }

    fn csr() -> (String, KeyPair) {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        // The agent may ask for anything: it is ignored.
        let mut params = CertificateParams::new(vec!["evil.example.com".to_string()]).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let csr = params.serialize_request(&key).unwrap();
        (csr.pem().unwrap(), key)
    }

    #[test]
    fn an_identity_survives_its_uri() {
        let id = identity();
        assert_eq!(
            id.to_uri(),
            "proxyauth://team-a/prod/6c1f3a52-0b8e-4b7f-9d2a-3e4f5a6b7c8d/3"
        );
        assert_eq!(AgentIdentity::from_uri(&id.to_uri()).unwrap(), id);
    }

    #[test]
    fn malformed_identities_are_refused() {
        for uri in [
            "https://team-a/prod/abc/0",
            "proxyauth://team-a/prod/abc",
            "proxyauth://team-a/prod/abc/0/extra",
            "proxyauth://Team-A/prod/abc/0",
            "proxyauth://team-a/prod/not-hex!/0",
            "proxyauth://team-a/prod/abc/-1",
            "proxyauth://team-a//abc/0",
        ] {
            assert!(AgentIdentity::from_uri(uri).is_err(), "{uri}");
        }
    }

    #[test]
    fn an_issued_agent_certificate_carries_only_the_identity_the_ca_chose() {
        let ca = AgentCa::generate().unwrap();
        let (csr, _) = csr();
        let pem = ca.issue_agent(&csr, &identity()).unwrap();
        let der = rcgen_der(&pem);
        let peer = identity_of(&der).unwrap();
        assert_eq!(peer.identity, identity());

        let (_, cert) = X509Certificate::from_der(&der).unwrap();
        assert!(!cert.is_ca(), "a CSR asking for CA rights is not obeyed");
        let san = cert.subject_alternative_name().unwrap().unwrap();
        assert_eq!(
            san.value.general_names.len(),
            1,
            "evil.example.com is dropped"
        );
        let lifetime = peer.not_after - OffsetDateTime::now_utc();
        assert!(lifetime <= time::Duration::hours(24) && lifetime > time::Duration::hours(23));
    }

    #[test]
    fn a_csr_with_a_bad_signature_is_refused() {
        let ca = AgentCa::generate().unwrap();
        let (csr, _) = csr();
        // Flip a byte of the base64 body near its end (the signature).
        let mut bytes = csr.into_bytes();
        let at = bytes.len().saturating_sub(40);
        if let Some(byte) = bytes.get_mut(at) {
            *byte = if *byte == b'A' { b'B' } else { b'A' };
        }
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(ca.issue_agent(&tampered, &identity()).is_err());
    }

    #[test]
    fn a_reloaded_ca_issues_certificates_its_original_verifies() {
        let ca = AgentCa::generate().unwrap();
        let reloaded = AgentCa::from_pem(ca.cert_pem(), ca.key_pem()).unwrap();
        let (csr, _) = csr();
        let pem = reloaded.issue_agent(&csr, &identity()).unwrap();
        let (der, ca_der) = (rcgen_der(&pem), rcgen_der(ca.cert_pem()));
        let (_, cert) = X509Certificate::from_der(&der).unwrap();
        let (_, ca_cert) = X509Certificate::from_der(&ca_der).unwrap();
        cert.verify_signature(Some(ca_cert.public_key())).unwrap();
    }

    #[test]
    fn a_server_certificate_names_the_tunnel_and_no_agent() {
        let ca = AgentCa::generate().unwrap();
        let (pem, key) = ca.issue_server().unwrap();
        assert!(key.contains("PRIVATE KEY"));
        assert!(matches!(
            identity_of(&rcgen_der(&pem)),
            Err(PkiError::NoIdentity)
        ));
    }

    #[test]
    fn the_ca_key_is_not_in_its_debug_output() {
        let ca = AgentCa::generate().unwrap();
        assert!(!format!("{ca:?}").contains("PRIVATE"));
    }

    fn rcgen_der(pem: &str) -> Vec<u8> {
        x509_parser::pem::parse_x509_pem(pem.as_bytes())
            .unwrap()
            .1
            .contents
    }
}
