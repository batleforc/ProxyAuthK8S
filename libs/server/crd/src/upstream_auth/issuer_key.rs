//! The `Issuer` signing keys of a cluster (RFC 0014 §5.3): the Secret
//! `proxyauth-issuer-<UID>` of the proxy's namespace, one `keys.json` holding
//! every key with its activation and retirement. The controller writes it
//! (rotation, revocation, purge), each proxy replica reads it to sign and to
//! publish the JWKS. Everything here is pure, so both sides agree on which key
//! is current without talking to each other.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Secret;
use kube::ResourceExt;
use serde::{Deserialize, Serialize};

use super::front_proxy::{MANAGED_BY_LABEL, MANAGED_BY_VALUE};
use crate::ProxyKubeApi;

/// The UID of the cluster a key Secret belongs to.
pub const CLUSTER_UID_LABEL: &str = "proxyauth.weebo.si.rs/cluster-uid";
/// The cluster's `ns/name`, for the people reading the Secret.
pub const CLUSTER_ANNOTATION: &str = "proxyauth.weebo.si.rs/cluster";
/// On the `ProxyKubeApi`: a new value rotates the key, `revoke:<nonce>`
/// revokes it (decisions 27 and 52).
pub const ROTATE_ANNOTATION: &str = "proxyauth.weebo.si.rs/rotate-issuer-key";
/// On the Secret: the last [`ROTATE_ANNOTATION`] value acted upon.
pub const PROCESSED_ANNOTATION: &str = "proxyauth.weebo.si.rs/rotate-issuer-key-processed";
/// On the Secret: when the sweep first found it without a cluster.
pub const ORPHANED_ANNOTATION: &str = "proxyauth.weebo.si.rs/orphaned-since";
/// The Secret's only data key.
pub const KEYS_KEY: &str = "keys.json";
/// A rotation's new key signs this long after it is published: the
/// replicas' 30 s cache, twice, plus the JWKS `max-age`, plus a margin.
pub const ACTIVATION_DELAY_SECONDS: i64 = 5 * 60;
/// How long a replica keeps `keys.json` before reading it again.
pub const KEYS_CACHE_SECONDS: i64 = 30;
/// The `Cache-Control: max-age` of the discovery and JWKS documents.
pub const DOCUMENT_MAX_AGE_SECONDS: i64 = 60;

/// One P-256 signing key. `pkcs8` is the private key (standard base64),
/// `x` and `y` its public point (base64url, as in a JWK).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct IssuerKey {
    pub kid: String,
    pub pkcs8: String,
    pub x: String,
    pub y: String,
    pub created_at: i64,
    pub activate_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retire_at: Option<i64>,
}

impl IssuerKey {
    fn live(&self, now: i64) -> bool {
        self.retire_at.is_none_or(|retire| retire > now)
    }
}

/// The content of `keys.json`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct IssuerKeySet {
    pub keys: Vec<IssuerKey>,
}

impl IssuerKeySet {
    /// The key that signs: the most recent one already active and not
    /// retired. Every replica picks it on its own (§5.3).
    #[must_use]
    pub fn current(&self, now: i64) -> Option<&IssuerKey> {
        self.keys
            .iter()
            .filter(|key| key.activate_at <= now && key.live(now))
            .max_by_key(|key| (key.activate_at, key.created_at))
    }

    /// The keys the JWKS publishes: every key not retired yet, the pending
    /// one included, so the apiserver can verify it the moment it signs.
    pub fn published(&self, now: i64) -> impl Iterator<Item = &IssuerKey> {
        self.keys.iter().filter(move |key| key.live(now))
    }

    /// The latest `retire_at` of the published keys, or `None` when one of
    /// them has none: how long a replica may serve this set without reading
    /// the Secret again (decision 44).
    #[must_use]
    pub fn serve_until(&self, now: i64) -> Option<i64> {
        let mut until = Some(i64::MIN);
        for key in self.published(now) {
            until = match (until, key.retire_at) {
                (Some(a), Some(b)) => Some(a.max(b)),
                _ => None,
            };
        }
        until
    }

    /// The next instant the set changes on its own: an activation or a
    /// retirement still ahead.
    #[must_use]
    pub fn next_change(&self, now: i64) -> Option<i64> {
        self.keys
            .iter()
            .flat_map(|key| [Some(key.activate_at), key.retire_at])
            .flatten()
            .filter(|at| *at > now)
            .min()
    }
}

/// What the rotation planner decided.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    /// Nothing to write.
    Keep,
    /// Write this set; `processed` is the rotation request acted upon.
    Write {
        keys: IssuerKeySet,
        processed: Option<String>,
    },
}

/// The settings the planner needs, from `spec.upstream_auth.issuer_key`.
#[derive(Clone, Copy, Debug)]
pub struct Rotation {
    pub rotation_days: u32,
    pub token_ttl_seconds: u32,
}

/// Decide the next `keys.json` (§5.3): purge what is retired, create the
/// first key active at once, rotate after `rotation_days` or on a new
/// [`ROTATE_ANNOTATION`] value (a new key active in
/// [`ACTIVATION_DELAY_SECONDS`], one pending key at a time), or revoke on a
/// new `revoke:<nonce>` (the active key gone at once, a new one active at
/// once). `new_key` makes a key, given its `created_at` and `activate_at`.
///
/// # Errors
///
/// The error of `new_key`; nothing is to be written then.
pub fn plan<E>(
    current: &IssuerKeySet,
    now: i64,
    rotation: Rotation,
    request: Option<&str>,
    processed: Option<&str>,
    new_key: &mut dyn FnMut(i64, i64) -> Result<IssuerKey, E>,
) -> Result<Plan, E> {
    let mut keys = current.clone();
    keys.keys.retain(|key| key.live(now));
    let fresh = request.filter(|value| Some(*value) != processed);
    let retire_after =
        i64::from(rotation.token_ttl_seconds).saturating_add(ACTIVATION_DELAY_SECONDS);

    if let Some(value) = fresh.filter(|value| value.starts_with("revoke:")) {
        let active = keys.current(now).map(|key| key.kid.clone());
        // A pending key goes too: it lives in the same Secret as the revoked
        // one, and would otherwise outlive the replacement, which then never
        // gets a `retire_at`.
        keys.keys
            .retain(|key| Some(&key.kid) != active.as_ref() && key.activate_at <= now);
        keys.keys.push(new_key(now, now)?);
        return Ok(Plan::Write {
            keys,
            processed: Some(value.to_string()),
        });
    }
    let pending = keys.keys.iter().any(|key| key.activate_at > now);
    // One pending key at a time: a request made meanwhile stays unprocessed,
    // and rotates once the pending key is active.
    let fresh = fresh.filter(|_| !pending);
    let due = keys.current(now).is_some_and(|key| {
        now >= key
            .activate_at
            .saturating_add(i64::from(rotation.rotation_days).saturating_mul(86_400))
    });
    if keys.current(now).is_none() && !pending {
        keys.keys.push(new_key(now, now)?);
    } else if !pending && (due || fresh.is_some()) {
        let activate_at = now.saturating_add(ACTIVATION_DELAY_SECONDS);
        for key in &mut keys.keys {
            if key.retire_at.is_none() {
                key.retire_at = Some(activate_at.saturating_add(retire_after));
            }
        }
        keys.keys.push(new_key(now, activate_at)?);
    } else if fresh.is_none() && keys == *current {
        return Ok(Plan::Keep);
    }
    Ok(Plan::Write {
        processed: fresh.or(processed).map(ToString::to_string),
        keys,
    })
}

/// Why a cluster's keys cannot be used. The messages name the Secret, never
/// its content.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IssuerKeyError {
    #[error("the ProxyKubeApi has no UID yet")]
    NoUid,
    #[error(
        "Secret {namespace}/{name} is not the proxy's (labels {MANAGED_BY_LABEL}={MANAGED_BY_VALUE} and {CLUSTER_UID_LABEL}=<UID> expected): it is never used, overwritten nor adopted; delete it so the controller creates the key"
    )]
    NotOwned { namespace: String, name: String },
    #[error("Secret {namespace}/{name} has no readable {KEYS_KEY}")]
    Unreadable { namespace: String, name: String },
}

/// `proxyauth-issuer-<UID>`: a recreated cluster gets a new key.
///
/// # Errors
///
/// [`IssuerKeyError::NoUid`] before the API server gave it one.
pub fn secret_name(proxy: &ProxyKubeApi) -> Result<String, IssuerKeyError> {
    proxy
        .uid()
        .map(|uid| format!("proxyauth-issuer-{uid}"))
        .ok_or(IssuerKeyError::NoUid)
}

/// The labels of a key Secret of the cluster `uid`.
#[must_use]
pub fn labels(uid: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (MANAGED_BY_LABEL.to_string(), MANAGED_BY_VALUE.to_string()),
        (CLUSTER_UID_LABEL.to_string(), uid.to_string()),
    ])
}

/// Whether `secret`, read from the proxy's namespace, is the key Secret of
/// the cluster `uid` (decision 43): both labels, nothing else.
#[must_use]
pub fn owned(secret: &Secret, uid: &str) -> bool {
    let labels = secret.labels();
    labels.get(MANAGED_BY_LABEL).map(String::as_str) == Some(MANAGED_BY_VALUE)
        && labels.get(CLUSTER_UID_LABEL).map(String::as_str) == Some(uid)
}

/// The key set of an owned Secret.
///
/// # Errors
///
/// [`IssuerKeyError::NotOwned`] or [`IssuerKeyError::Unreadable`].
pub fn read(secret: &Secret, uid: &str) -> Result<IssuerKeySet, IssuerKeyError> {
    let namespace = secret.namespace().unwrap_or_default();
    let name = secret.name_any();
    if !owned(secret, uid) {
        return Err(IssuerKeyError::NotOwned { namespace, name });
    }
    secret
        .data
        .as_ref()
        .and_then(|data| data.get(KEYS_KEY))
        .and_then(|bytes| serde_json::from_slice(&bytes.0).ok())
        .ok_or(IssuerKeyError::Unreadable { namespace, name })
}

/// The path `issuer_url` must end with: the route the proxy serves.
#[must_use]
pub fn issuer_path(proxy: &ProxyKubeApi) -> String {
    format!(
        "/issuer/{}/{}",
        proxy.namespace().unwrap_or_default(),
        proxy.name_any()
    )
}

/// Whether `issuer_url` names this cluster's route (`IssuerUrlMismatch`
/// otherwise); a trailing `/` is allowed, the apiserver trims it to build
/// the discovery URL.
#[must_use]
pub fn issuer_url_matches(proxy: &ProxyKubeApi, issuer_url: &str) -> bool {
    issuer_url
        .trim_end_matches('/')
        .ends_with(&issuer_path(proxy))
}

/// The JWKS URL published in the discovery document.
#[must_use]
pub fn jwks_uri(issuer_url: &str) -> String {
    format!("{}/jwks", issuer_url.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROTATION: Rotation = Rotation {
        rotation_days: 30,
        token_ttl_seconds: 300,
    };

    fn maker() -> impl FnMut(i64, i64) -> Result<IssuerKey, ()> {
        let mut n: u32 = 0;
        move |created_at, activate_at| {
            n = n.saturating_add(1);
            Ok(IssuerKey {
                kid: format!("k{n}"),
                pkcs8: String::new(),
                x: String::new(),
                y: String::new(),
                created_at,
                activate_at,
                retire_at: None,
            })
        }
    }

    fn written(plan: Result<Plan, ()>) -> (IssuerKeySet, Option<String>) {
        match plan {
            Ok(Plan::Write { keys, processed }) => (keys, processed),
            _ => panic!("expected a write"),
        }
    }

    #[test]
    fn the_first_key_is_active_at_once() {
        let mut new_key = maker();
        let (keys, _) = written(plan(
            &IssuerKeySet::default(),
            1_000,
            ROTATION,
            None,
            None,
            &mut new_key,
        ));
        assert_eq!(keys.current(1_000).map(|k| k.kid.as_str()), Some("k1"));
        let again = plan(&keys, 1_001, ROTATION, None, None, &mut new_key);
        assert_eq!(again, Ok(Plan::Keep));
    }

    #[test]
    fn a_rotation_publishes_before_it_signs_and_retires_the_old_key_later() {
        let mut new_key = maker();
        let (first, _) = written(plan(
            &IssuerKeySet::default(),
            0,
            ROTATION,
            None,
            None,
            &mut new_key,
        ));
        let due = 30 * 86_400;
        let (rotated, _) = written(plan(&first, due, ROTATION, None, None, &mut new_key));
        // Published at once, signing only after the delay.
        assert_eq!(rotated.published(due).count(), 2);
        assert_eq!(rotated.current(due).map(|k| k.kid.as_str()), Some("k1"));
        let active = due + ACTIVATION_DELAY_SECONDS;
        assert_eq!(rotated.current(active).map(|k| k.kid.as_str()), Some("k2"));
        // The old key stays published for the longest-lived JWT it signed.
        let retire = active + 300 + ACTIVATION_DELAY_SECONDS;
        assert_eq!(rotated.published(retire - 1).count(), 2);
        assert_eq!(rotated.published(retire).count(), 1);
        // One pending key at a time: a second request waits, unprocessed,
        // then rotates once the pending key is active.
        let wait = plan(
            &rotated,
            due + 1,
            ROTATION,
            Some("again"),
            None,
            &mut new_key,
        );
        assert_eq!(wait, Ok(Plan::Keep));
        let (again, processed) = written(plan(
            &rotated,
            active,
            ROTATION,
            Some("again"),
            None,
            &mut new_key,
        ));
        assert_eq!(again.keys.len(), 3);
        assert_eq!(processed.as_deref(), Some("again"));
        // Purged once retired: a restored backup brings nothing back.
        let (purged, _) = written(plan(&rotated, retire, ROTATION, None, None, &mut new_key));
        assert_eq!(purged.keys.len(), 1);
    }

    #[test]
    fn the_delay_covers_two_replica_caches_and_the_document_max_age() {
        const {
            assert!(
                ACTIVATION_DELAY_SECONDS >= 2 * KEYS_CACHE_SECONDS + DOCUMENT_MAX_AGE_SECONDS + 60
            );
        }
    }

    #[test]
    fn a_rotation_request_is_acted_upon_once() {
        let mut new_key = maker();
        let (first, _) = written(plan(
            &IssuerKeySet::default(),
            0,
            ROTATION,
            None,
            None,
            &mut new_key,
        ));
        let (rotated, processed) = written(plan(
            &first,
            10,
            ROTATION,
            Some("2026-10-29"),
            None,
            &mut new_key,
        ));
        assert_eq!(rotated.keys.len(), 2);
        assert_eq!(processed.as_deref(), Some("2026-10-29"));
        let after = rotated.current(10 + ACTIVATION_DELAY_SECONDS).cloned();
        let same = plan(
            &rotated,
            10 + ACTIVATION_DELAY_SECONDS,
            ROTATION,
            Some("2026-10-29"),
            Some("2026-10-29"),
            &mut new_key,
        );
        assert_eq!(same, Ok(Plan::Keep));
        assert!(after.is_some());
    }

    /// Decision 52: the active key leaves the set at once, a new key signs
    /// at once with a `kid` the apiserver does not know.
    #[test]
    fn a_revocation_removes_the_active_key_at_once() {
        let mut new_key = maker();
        let (first, _) = written(plan(
            &IssuerKeySet::default(),
            0,
            ROTATION,
            None,
            None,
            &mut new_key,
        ));
        let (revoked, processed) = written(plan(
            &first,
            50,
            ROTATION,
            Some("revoke:a1"),
            None,
            &mut new_key,
        ));
        assert!(revoked.keys.iter().all(|k| k.kid != "k1"));
        assert_eq!(revoked.current(50).map(|k| k.kid.as_str()), Some("k2"));
        assert_eq!(processed.as_deref(), Some("revoke:a1"));
        let replay = plan(
            &revoked,
            60,
            ROTATION,
            Some("revoke:a1"),
            Some("revoke:a1"),
            &mut new_key,
        );
        assert_eq!(replay, Ok(Plan::Keep));
    }

    /// A revocation during a rotation drops the pending key too, so the
    /// replacement is the only one left to sign and is rotated normally.
    #[test]
    fn a_revocation_during_a_rotation_leaves_one_key() {
        let mut new_key = maker();
        let (first, _) = written(plan(
            &IssuerKeySet::default(),
            0,
            ROTATION,
            None,
            None,
            &mut new_key,
        ));
        let (rotating, _) = written(plan(&first, 10, ROTATION, Some("r1"), None, &mut new_key));
        let (revoked, _) = written(plan(
            &rotating,
            20,
            ROTATION,
            Some("revoke:a1"),
            Some("r1"),
            &mut new_key,
        ));
        let kids: Vec<_> = revoked.keys.iter().map(|k| k.kid.as_str()).collect();
        assert_eq!(kids, ["k3"]);
        assert_eq!(revoked.current(20).map(|k| k.kid.as_str()), Some("k3"));
    }

    #[test]
    fn a_set_is_served_until_its_last_key_retires() {
        let key = |retire_at| IssuerKey {
            kid: String::new(),
            pkcs8: String::new(),
            x: String::new(),
            y: String::new(),
            created_at: 0,
            activate_at: 0,
            retire_at,
        };
        let set = IssuerKeySet {
            keys: vec![key(Some(100)), key(Some(200))],
        };
        assert_eq!(set.serve_until(0), Some(200));
        let open = IssuerKeySet {
            keys: vec![key(Some(100)), key(None)],
        };
        assert_eq!(open.serve_until(0), None);
        assert_eq!(set.next_change(0), Some(100));
    }

    fn secret(labels: &[(&str, &str)]) -> Secret {
        let mut secret = Secret::default();
        secret.metadata.name = Some("proxyauth-issuer-u1".into());
        secret.metadata.labels = Some(
            labels
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        );
        let keys = serde_json::to_vec(&IssuerKeySet::default()).unwrap();
        secret.data = Some(BTreeMap::from([(
            KEYS_KEY.to_string(),
            k8s_openapi::ByteString(keys),
        )]));
        secret
    }

    /// T18: a Secret with the right name but not both labels, or the labels
    /// of another cluster, is never used.
    #[test]
    fn a_secret_without_the_expected_labels_is_never_adopted() {
        let good = secret(&[
            (MANAGED_BY_LABEL, MANAGED_BY_VALUE),
            (CLUSTER_UID_LABEL, "u1"),
        ]);
        assert!(read(&good, "u1").is_ok());
        assert!(matches!(
            read(&good, "u2"),
            Err(IssuerKeyError::NotOwned { .. })
        ));
        let unlabelled = secret(&[(CLUSTER_UID_LABEL, "u1")]);
        assert!(matches!(
            read(&unlabelled, "u1"),
            Err(IssuerKeyError::NotOwned { .. })
        ));
    }

    #[test]
    fn the_issuer_url_must_name_the_route_of_its_cluster() {
        let mut proxy = ProxyKubeApi::new(
            "prod",
            serde_json::from_value(serde_json::json!({
                "cert": { "SystemRoots": true },
                "service": { "ExternalService": { "url": "https://k:6443" } }
            }))
            .unwrap(),
        );
        proxy.metadata.namespace = Some("team-a".into());
        assert!(issuer_url_matches(
            &proxy,
            "https://p.example.com/issuer/team-a/prod"
        ));
        assert!(issuer_url_matches(
            &proxy,
            "https://p.example.com/issuer/team-a/prod/"
        ));
        assert!(!issuer_url_matches(
            &proxy,
            "https://p.example.com/issuer/team-b/prod"
        ));
        assert_eq!(
            jwks_uri("https://p.example.com/issuer/team-a/prod/"),
            "https://p.example.com/issuer/team-a/prod/jwks"
        );
    }
}
