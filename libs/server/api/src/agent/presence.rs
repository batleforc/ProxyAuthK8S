//! This gateway pod's presences, and the revocation of its tunnels (RFC 0012
//! §6.3, T8).
//!
//! Every [`PRESENCE_REFRESH`], each cluster this pod holds a tunnel for is
//! checked again against its `ProxyKubeApi`: still there, same UID, still an
//! `Agent` service, `epoch` not raised. A cluster that passes gets this pod's
//! name in its presence set, which the proxy reads to route a `CONNECT`, and
//! a record of its tunnels the controller reads for `status.agent`; one
//! that fails loses it, and its tunnels are closed. A Redis failure changes
//! nothing: the presences expire on their own, the tunnels stay up.

use std::collections::HashMap;
use std::sync::Arc;

use common::agent_gateway::{
    PRESENCE_REFRESH, PRESENCE_TTL, PresenceInfo, presence_info_key, presence_key,
};
use tracing::{info, warn};

use super::listener::{Gateway, current_agent};

/// Refresh the presences and close the revoked tunnels, forever.
pub async fn run(gateway: Arc<Gateway>) {
    let mut ticks = tokio::time::interval(PRESENCE_REFRESH);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        refresh(&gateway).await;
    }
}

/// One round of [`run`].
pub async fn refresh(gateway: &Gateway) {
    // uid -> the CR's epoch, or `None` once the CR no longer accepts the UID.
    let mut current: HashMap<String, Option<u64>> = HashMap::new();
    for (identity, info) in gateway.registry.identities() {
        let key = presence_key(&identity.uid);
        let epoch = match current_agent(&gateway.state, &identity).await {
            Ok(agent) => Some(agent.epoch),
            Err("redis") => continue,
            Err(reason) => {
                info!(cluster = %identity.cluster(), reason, "agent tunnels revoked");
                None
            }
        };
        let published = match epoch {
            Some(_) => publish(gateway, &identity.uid, &info).await,
            None => gateway.state.presence_remove(&key, &gateway.pod).await,
        };
        if let Err(err) = published {
            warn!(cluster = %identity.cluster(), error = %err, "agent presence not published");
        }
        current.insert(identity.uid, epoch);
    }
    // Per tunnel: two tunnels of one UID may carry certificates of two epochs.
    gateway
        .registry
        .close_where(|identity| match current.get(&identity.uid) {
            Some(Some(epoch)) => *epoch > identity.epoch,
            Some(None) => true,
            None => false,
        });
}

/// This pod's record for `status.agent`, then its member in the presence set
/// the proxy routes by: a member never lacks its record for long.
async fn publish(
    gateway: &Gateway,
    uid: &str,
    info: &PresenceInfo,
) -> Result<(), common::redis_pool::RedisPoolError> {
    let record = serde_json::to_string(info).unwrap_or_default();
    gateway
        .state
        .redis_set(
            &presence_info_key(uid, &gateway.pod),
            &record,
            Some(PRESENCE_TTL.as_secs()),
        )
        .await?;
    gateway
        .state
        .presence_refresh(&presence_key(uid), &gateway.pod, PRESENCE_TTL.as_secs())
        .await
}

/// At shutdown: withdraw this pod from every presence set and close its
/// tunnels, so the proxy and the agents move to another pod at once.
pub async fn withdraw(gateway: &Gateway) {
    for (identity, _) in gateway.registry.identities() {
        let key = presence_key(&identity.uid);
        if let Err(err) = gateway.state.presence_remove(&key, &gateway.pod).await {
            warn!(cluster = %identity.cluster(), error = %err, "agent presence not withdrawn");
        }
    }
    gateway.registry.close_where(|_| true);
}
