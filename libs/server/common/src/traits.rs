pub trait ObjectRedis {
    fn from_json(json: &str) -> Option<Self>
    where
        Self: Sized;
    fn to_json(&self) -> String;
}
