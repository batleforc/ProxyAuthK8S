use openidconnect::{core::CoreGenderClaim, AdditionalClaims, UserInfoClaims};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct GroupsAdditionalClaims {
    pub groups: Vec<String>,
}
impl AdditionalClaims for GroupsAdditionalClaims {}

pub type GroupsUserInfoClaims = UserInfoClaims<GroupsAdditionalClaims, CoreGenderClaim>;
