use openidconnect::{AdditionalClaims, UserInfoClaims, core::CoreGenderClaim};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct GroupsAdditionalClaims {
    /// Groups the provider asserted for this user, empty when it asserted none.
    ///
    /// Defaulted rather than required: providers tag the claim `omitempty` (Dex
    /// and Authentik both do), so a user who belongs to no group gets a
    /// `/userinfo` payload with no `groups` key at all. Treating that as a parse
    /// failure locked such users out of every cluster, including ones with no
    /// group restriction.
    ///
    /// This does not weaken authorization: [`crd::ProxyKubeApi::is_proxy_allowed`]
    /// matches the caller's groups against `proxy_group`, and an empty list
    /// matches nothing — a group-restricted cluster still refuses the request.
    #[serde(default)]
    pub groups: Vec<String>,
}
impl AdditionalClaims for GroupsAdditionalClaims {}

pub type GroupsUserInfoClaims = UserInfoClaims<GroupsAdditionalClaims, CoreGenderClaim>;
