//! Guard (RFC 0015 §6.3): every variant of an enum that drives behaviour names
//! the test that covers it. The `match`es have no `_` arm, so a new variant
//! stops this file from compiling until its line is written; then the named
//! test must exist in the named file.
//!
//! Adding a variant? Add an arm naming the test of the new variant, in the
//! file where it lives (a path from the repository root).

#![expect(
    clippy::expect_used,
    reason = "test helpers: a panic is a failed test, with its message"
)]

use crd::security::Verb;
use crd::upstream_auth::{UpstreamAuthMode, UpstreamTrust};

/// `(file from the repository root, test function)`.
type Covered = (&'static str, &'static str);

const UPSTREAM: &str = "libs/server/api/src/cluster/redirect/kube_redirect/upstream.rs";
const PROBE: &str = "libs/server/api/src/upstream_probe.rs";
const REQUEST_INFO: &str = "libs/server/crd/src/security/request_info.rs";
const SECURITY: &str = "libs/server/crd/src/security/mod.rs";

fn upstream_auth_mode(mode: UpstreamAuthMode) -> Covered {
    // Add an arm naming the test of the new variant.
    match mode {
        UpstreamAuthMode::Passthrough => (
            UPSTREAM,
            "passthrough_relays_the_token_and_never_a_client_identity_header",
        ),
        UpstreamAuthMode::FrontProxy => (
            UPSTREAM,
            "front_proxy_strips_every_authorization_value_and_asserts_the_identity",
        ),
        UpstreamAuthMode::Issuer => (UPSTREAM, "issuer_replaces_the_token_with_the_proxy_jwt"),
        UpstreamAuthMode::TokenExchange => {
            (UPSTREAM, "client_authorization_headers_are_all_replaced")
        }
    }
}

fn verb(verb: Verb) -> Covered {
    // Add an arm naming the test of the new variant.
    match verb {
        Verb::Get
        | Verb::List
        | Verb::Watch
        | Verb::Create
        | Verb::Update
        | Verb::Patch
        | Verb::Delete
        | Verb::Deletecollection => (REQUEST_INFO, "the_verb_table"),
        Verb::All => (SECURITY, "star_allows_every_verb"),
    }
}

fn upstream_trust(trust: UpstreamTrust) -> Covered {
    // Add an arm naming the test of the new variant.
    match trust {
        UpstreamTrust::Trusted => (PROBE, "the_asserted_identity_seen_whole_is_trusted"),
        UpstreamTrust::NotHonoured => (PROBE, "another_identity_or_a_lost_header_is_not_honoured"),
        UpstreamTrust::Rejected | UpstreamTrust::Unknown => {
            (PROBE, "a_401_is_rejected_and_anything_else_unknown")
        }
        UpstreamTrust::Unguarded => (PROBE, "an_accepted_unprefixed_identity_is_unguarded"),
    }
}

/// The repository root, from this crate's manifest directory.
fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("the repository root resolves")
}

/// What is wrong with `covered`: a file that cannot be read, or a test that is
/// not declared in it.
fn missing(covered: &[Covered]) -> Vec<String> {
    let root = repo_root();
    covered
        .iter()
        .filter_map(|(file, test)| {
            let Ok(source) = std::fs::read_to_string(root.join(file)) else {
                return Some(format!("variant_coverage: {file} cannot be read"));
            };
            (!source.contains(&format!("fn {test}(")))
                .then(|| format!("variant_coverage: {file} has no test named {test}"))
        })
        .collect()
}

#[test]
fn every_variant_names_a_test_that_exists() {
    let mut covered = Vec::new();
    covered.extend(
        crd::upstream_auth::SERVED_MODES
            .iter()
            .map(|m| upstream_auth_mode(*m)),
    );
    covered.extend(
        [
            Verb::Get,
            Verb::List,
            Verb::Watch,
            Verb::Create,
            Verb::Update,
            Verb::Patch,
            Verb::Delete,
            Verb::Deletecollection,
            Verb::All,
        ]
        .map(verb),
    );
    covered.extend(
        [
            UpstreamTrust::Trusted,
            UpstreamTrust::NotHonoured,
            UpstreamTrust::Rejected,
            UpstreamTrust::Unknown,
            UpstreamTrust::Unguarded,
        ]
        .map(upstream_trust),
    );
    let problems = missing(&covered);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The guard can fail: a test name that no file declares is reported.
#[test]
fn a_test_that_does_not_exist_is_reported() {
    let problems = missing(&[(PROBE, "no_such_test")]);
    assert_eq!(
        problems,
        [format!(
            "variant_coverage: {PROBE} has no test named no_such_test"
        )]
    );
}
