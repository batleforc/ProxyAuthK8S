//! The enrolment body (RFC 0012 §5.2) on hostile bytes: parsing never panics,
//! and a hostile CSR is refused or signed for the identity the gateway
//! chose, never one the CSR names.

#![no_main]

use std::sync::LazyLock;

use agent_pki::{AgentCa, AgentIdentity, identity_of};
use libfuzzer_sys::fuzz_target;
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject as _;

static CA: LazyLock<AgentCa> = LazyLock::new(|| AgentCa::generate().expect("a CA"));

fuzz_target!(|data: &[u8]| {
    let identity = AgentIdentity {
        namespace: "team-a".to_string(),
        name: "prod".to_string(),
        uid: "aa".to_string(),
        epoch: 3,
    };
    let csrs = match api::agent::enroll::parse_body(data) {
        Some(request) => vec![request.csr],
        None => vec![String::from_utf8_lossy(data).into_owned()],
    };
    for csr in csrs {
        let Ok(cert) = CA.issue_agent(&csr, &identity) else { continue };
        let der = CertificateDer::from_pem_slice(cert.as_bytes()).expect("PEM out");
        assert_eq!(identity_of(&der).expect("an identity").identity, identity);
    }
});
