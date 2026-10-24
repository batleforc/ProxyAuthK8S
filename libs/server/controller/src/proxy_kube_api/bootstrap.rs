//! The `OneShotToken` of a `service: Agent` cluster (RFC 0012 §4.1, §6.4).
//!
//! The controller writes the token to the Secret `<name>-agent-bootstrap`
//! next to the `ProxyKubeApi`, and marks it usable in Redis by its hash for
//! one hour; the gateway takes that mark at enrolment (`GETDEL`). A new token
//! replaces it when there is none, when it is used or expired and
//! `auto_rotate` is set, or when the CR's `regenerate-bootstrap` annotation
//! carries a value the Secret has not served yet.

use std::collections::BTreeMap;

use base64::Engine as _;
use common::State;
use common::agent_gateway::{BOOTSTRAP_TTL, bootstrap_key};
use crd::ProxyKubeApi;
use crd::service::{OneShotTokenBootstrap, REGENERATE_BOOTSTRAP_ANNOTATION, bootstrap_secret_name};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, ObjectMeta, Patch, PatchParams};
use kube::{Resource as _, ResourceExt as _};
use tracing::info;

use crate::error::{ControllerError, Result};
use crate::proxy_kube_api::FIELD_MANAGER;

/// The key of the token in the bootstrap Secret.
pub const TOKEN_KEY: &str = "token";

/// Whether the Secret's token, as found, must be replaced.
#[must_use]
pub fn must_regenerate(
    bootstrap: &OneShotTokenBootstrap,
    token: Option<&str>,
    usable: bool,
    asked: Option<&str>,
    served: Option<&str>,
) -> bool {
    token.is_none() || (bootstrap.auto_rotate && !usable) || (asked.is_some() && asked != served)
}

/// Write a new token when [`must_regenerate`] says so. Returns whether the
/// Secret now holds a usable token.
///
/// # Errors
///
/// [`ControllerError::Kube`] when the Secret cannot be read or written,
/// [`ControllerError::Redis`] when the mark cannot.
pub async fn ensure_one_shot_token(
    proxy: &ProxyKubeApi,
    bootstrap: &OneShotTokenBootstrap,
    ctx: &State,
) -> Result<bool> {
    let (Some(namespace), Some(uid)) = (proxy.namespace(), proxy.metadata.uid.as_deref()) else {
        return Ok(false);
    };
    let name = bootstrap_secret_name(&proxy.name_any());
    let secrets: Api<Secret> = Api::namespaced(ctx.client.clone(), &namespace);
    let current = secrets
        .get_opt(&name)
        .await
        .map_err(ControllerError::Kube)?;
    let token = current
        .as_ref()
        .and_then(|secret| secret.data.as_ref()?.get(TOKEN_KEY))
        .and_then(|token| String::from_utf8(token.0.clone()).ok());
    let usable = match &token {
        Some(token) => ctx.key_exists(&bootstrap_key(uid, token)).await?,
        None => false,
    };
    let asked = proxy
        .annotations()
        .get(REGENERATE_BOOTSTRAP_ANNOTATION)
        .map(String::as_str);
    let served = current
        .as_ref()
        .and_then(|secret| secret.annotations().get(REGENERATE_BOOTSTRAP_ANNOTATION))
        .map(String::as_str);
    if !must_regenerate(bootstrap, token.as_deref(), usable, asked, served) {
        return Ok(usable);
    }

    if let Some(old) = &token {
        ctx.delete_key(&bootstrap_key(uid, old)).await?;
    }
    let mut random = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut random).map_err(
        |_| ControllerError::InvalidResource("no randomness for the bootstrap token".to_string()),
    )?;
    let new = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random);
    // Marked first: a Secret written without its mark would hold a dead token.
    ctx.set_flag(&bootstrap_key(uid, &new), BOOTSTRAP_TTL.as_secs())
        .await?;
    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(namespace.clone()),
            annotations: asked.map(|asked| {
                BTreeMap::from([(
                    REGENERATE_BOOTSTRAP_ANNOTATION.to_string(),
                    asked.to_string(),
                )])
            }),
            owner_references: proxy.controller_owner_ref(&()).map(|owner| vec![owner]),
            ..ObjectMeta::default()
        },
        data: Some(BTreeMap::from([(
            TOKEN_KEY.to_string(),
            ByteString(new.into_bytes()),
        )])),
        type_: Some("Opaque".to_string()),
        ..Secret::default()
    };
    secrets
        .patch(
            &name,
            &PatchParams::apply(FIELD_MANAGER).force(),
            &Patch::Apply(&secret),
        )
        .await
        .map_err(ControllerError::Kube)?;
    info!(%namespace, secret = %name, "new agent bootstrap token");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_replaced_only_when_missing_spent_and_rotating_or_asked() {
        let once = OneShotTokenBootstrap { auto_rotate: false };
        let rotating = OneShotTokenBootstrap { auto_rotate: true };
        assert!(must_regenerate(&once, None, false, None, None));
        assert!(!must_regenerate(&once, Some("t"), true, None, None));
        // Spent: kept without auto_rotate, the operator asks for another.
        assert!(!must_regenerate(&once, Some("t"), false, None, None));
        assert!(must_regenerate(&rotating, Some("t"), false, None, None));
        assert!(!must_regenerate(&rotating, Some("t"), true, None, None));
        assert!(must_regenerate(
            &once,
            Some("t"),
            true,
            Some("2"),
            Some("1")
        ));
        assert!(!must_regenerate(
            &once,
            Some("t"),
            false,
            Some("2"),
            Some("2")
        ));
    }
}
