pub trait ObjectRedis {
    fn from_json(json: &str) -> Option<Self>
    where
        Self: Sized;
    fn to_json(&self) -> String;
}

// The CRD schema lives in `crd` (kept free of `common` so `crdgen` doesn't link
// Redis). The orphan rule forces this impl into the crate owning the trait.
impl ObjectRedis for crd::ProxyKubeApi {
    fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }
}
