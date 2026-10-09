use std::ops::RangeInclusive;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// CEL rule mirroring [`PortSpec::range`]: the pattern only bounds the digits,
/// so the 1-65535 bounds and the range order are checked here. A value that is
/// not digits makes `int()` fail, which is a rejection too.
const PORT_SPEC_RULE: &str = concat!(
    "self.split('-').all(p, int(p) >= 1 && int(p) <= 65535)",
    " && (!self.contains('-') || int(self.split('-')[0]) <= int(self.split('-')[1]))"
);

/// A port (`"8080"`) or an inclusive port range (`"9000-9100"`).
///
/// Ports must be within 1-65535 and a range must not be reversed. Admission
/// refuses an entry breaking that; one stored before the rule existed is
/// reported at reconcile time and allows no port.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
#[serde(transparent)]
#[schemars(extend("x-kubernetes-validations" = [
    serde_json::json!({
        "rule": PORT_SPEC_RULE,
        "message": "ports must be within 1-65535 and a range must not be reversed",
    }),
]))]
pub struct PortSpec(
    #[schemars(length(max = 11), regex(pattern = r"^[0-9]{1,5}(-[0-9]{1,5})?$"))] pub String,
);

impl PortSpec {
    /// The ports this entry allows.
    pub fn range(&self) -> Result<RangeInclusive<u16>, String> {
        let parse = |value: &str| {
            // Digits only: `u16::from_str` would also accept a leading `+`.
            Some(value)
                .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|v| v.parse::<u16>().ok())
                .filter(|port| *port > 0)
                .ok_or_else(|| format!("invalid port \"{value}\" in \"{}\"", self.0))
        };

        let (start, end) = match self.0.split_once('-') {
            Some((start, end)) => (parse(start)?, parse(end)?),
            None => {
                let port = parse(&self.0)?;
                (port, port)
            }
        };
        if start > end {
            return Err(format!("reversed port range \"{}\"", self.0));
        }
        Ok(start..=end)
    }
}

/// Which ports a port-forward session may open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortPolicy {
    /// No restriction.
    Any,
    /// Only these ports. An empty list allows none.
    Only(Vec<RangeInclusive<u16>>),
}

impl PortPolicy {
    #[must_use]
    pub fn allows(&self, port: u16) -> bool {
        match self {
            PortPolicy::Any => true,
            PortPolicy::Only(ranges) => ranges.iter().any(|range| range.contains(&port)),
        }
    }

    #[must_use]
    pub fn is_restricted(&self) -> bool {
        matches!(self, PortPolicy::Only(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(value: &str) -> PortSpec {
        PortSpec(value.to_string())
    }

    #[test]
    fn a_single_port_is_a_one_port_range() {
        assert_eq!(spec("8080").range(), Ok(8080..=8080));
    }

    #[test]
    fn a_range_is_inclusive() {
        assert_eq!(spec("9000-9100").range(), Ok(9000..=9100));
    }

    #[test]
    fn out_of_bounds_and_malformed_ports_are_rejected() {
        for value in [
            "0", "65536", "99999", "10-0", "200-100", "-80", "80-", "", "a", "+80",
        ] {
            assert!(spec(value).range().is_err(), "{value} should be rejected");
        }
    }

    #[test]
    fn an_empty_only_policy_allows_nothing() {
        assert!(!PortPolicy::Only(Vec::new()).allows(8080));
        assert!(PortPolicy::Any.allows(8080));
        let policy = PortPolicy::Only(vec![80..=90, 443..=443]);
        assert!(policy.allows(85));
        assert!(policy.allows(443));
        assert!(!policy.allows(91));
    }
}
