//! Generic Kubernetes-style list envelope for CLI `get` output.
//!
//! The `get` commands all render a list of items as JSON, YAML, or a table.
//! Rather than repeat that envelope and rendering once per item type, each item
//! implements [`TableRow`] and reuses the single generic [`KubeList`].

use comfy_table::Table;
use serde::{Deserialize, Serialize};

use crate::ctx::ContextFormat;

/// An item that can be rendered as one row of a CLI table.
pub trait TableRow {
    /// Column headers, in table-column order.
    fn headers() -> Vec<String>;
    /// This item's cells, in the same order as [`TableRow::headers`].
    fn row(&self) -> Vec<String>;
}

/// A Kubernetes-style `List` envelope over any renderable item type.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KubeList<T> {
    /// Serialized as `apiVersion`, like every Kubernetes object.
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub metadata: Option<serde_json::Value>,
    pub items: Vec<T>,
}

impl<T> KubeList<T>
where
    T: Serialize + TableRow,
{
    /// Wrap `items` in a `v1`/`List` envelope.
    #[must_use]
    pub fn new(items: Vec<T>) -> Self {
        Self {
            api_version: "v1".to_string(),
            kind: "List".to_string(),
            metadata: None,
            items,
        }
    }

    /// Render as pretty-printed JSON (empty string on serialization failure).
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Render as YAML (empty string on serialization failure).
    #[must_use]
    pub fn to_yaml(&self) -> String {
        serde_yaml_ng::to_string(self).unwrap_or_default()
    }

    /// Render as a borderless text table.
    #[must_use]
    pub fn to_table(&self) -> String {
        let mut table = Table::new();
        table.load_style(comfy_table::presets::NOTHING);
        table.set_header(T::headers());
        for item in &self.items {
            table.add_row(item.row());
        }
        table.to_string()
    }

    /// Render in the caller-selected [`ContextFormat`].
    #[must_use]
    pub fn to_output(&self, format: ContextFormat) -> String {
        match format {
            ContextFormat::Json => self.to_json(),
            ContextFormat::Yaml => self.to_yaml(),
            ContextFormat::Table => self.to_table(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct Item {
        name: String,
        ready: bool,
    }

    impl TableRow for Item {
        fn headers() -> Vec<String> {
            vec!["NAME".to_string(), "READY".to_string()]
        }

        fn row(&self) -> Vec<String> {
            vec![self.name.clone(), self.ready.to_string()]
        }
    }

    fn list() -> KubeList<Item> {
        KubeList::new(vec![
            Item {
                name: "alpha".to_string(),
                ready: true,
            },
            Item {
                name: "beta".to_string(),
                ready: false,
            },
        ])
    }

    #[test]
    fn new_wraps_items_in_a_v1_list_envelope() {
        let list = list();
        assert_eq!(list.api_version, "v1");
        assert_eq!(list.kind, "List");
        assert!(list.metadata.is_none());
        assert_eq!(list.items.len(), 2);
    }

    #[test]
    fn json_output_is_the_serialized_envelope() {
        let value: serde_json::Value = serde_json::from_str(&list().to_json()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "apiVersion": "v1",
                "kind": "List",
                "metadata": null,
                "items": [
                    {"name": "alpha", "ready": true},
                    {"name": "beta", "ready": false},
                ],
            })
        );
    }

    #[test]
    fn yaml_output_round_trips_to_the_same_document() {
        let yaml: serde_json::Value = serde_yaml_ng::from_str(&list().to_yaml()).unwrap();
        let json: serde_json::Value = serde_json::from_str(&list().to_json()).unwrap();
        assert_eq!(yaml, json);
    }

    #[test]
    fn table_output_has_a_header_then_one_line_per_item() {
        let table = list().to_table();
        let lines: Vec<Vec<&str>> = table
            .lines()
            .map(|line| line.split_whitespace().collect())
            .collect();
        assert_eq!(
            lines,
            [
                vec!["NAME", "READY"],
                vec!["alpha", "true"],
                vec!["beta", "false"]
            ]
        );
    }

    #[test]
    fn empty_table_still_prints_the_header() {
        let table = KubeList::<Item>::new(vec![]).to_table();
        assert_eq!(
            table.split_whitespace().collect::<Vec<_>>(),
            ["NAME", "READY"]
        );
    }

    #[test]
    fn to_output_dispatches_on_the_format() {
        let list = list();
        assert_eq!(list.to_output(ContextFormat::Json), list.to_json());
        assert_eq!(list.to_output(ContextFormat::Yaml), list.to_yaml());
        assert_eq!(list.to_output(ContextFormat::Table), list.to_table());
    }
}
