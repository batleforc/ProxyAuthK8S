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

    #[derive(Serialize, Deserialize, Debug, Clone)]
    struct Row {
        name: String,
        count: u8,
    }

    impl TableRow for Row {
        fn headers() -> Vec<String> {
            vec!["NAME".to_string(), "COUNT".to_string()]
        }

        fn row(&self) -> Vec<String> {
            vec![self.name.clone(), self.count.to_string()]
        }
    }

    fn rows() -> Vec<Row> {
        vec![
            Row {
                name: "alpha".to_string(),
                count: 1,
            },
            Row {
                name: "beta".to_string(),
                count: 2,
            },
        ]
    }

    #[test]
    fn new_wraps_items_in_a_v1_list_envelope() {
        let list = KubeList::new(rows());
        assert_eq!(list.api_version, "v1");
        assert_eq!(list.kind, "List");
        assert!(list.metadata.is_none());
        assert_eq!(list.items.len(), 2);
    }

    #[test]
    fn to_json_is_pretty_printed_and_round_trips() {
        let json = KubeList::new(rows()).to_json();
        // Pretty-printed, so the envelope spans several lines.
        assert!(json.contains('\n'));
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(parsed["apiVersion"], "v1");
        assert!(parsed.get("api_version").is_none());
        assert_eq!(parsed["kind"], "List");
        assert_eq!(parsed["metadata"], serde_json::Value::Null);
        assert_eq!(parsed["items"][0]["name"], "alpha");
        assert_eq!(parsed["items"][1]["count"], 2);
    }

    #[test]
    fn to_yaml_round_trips_through_the_same_envelope() {
        let yaml = KubeList::new(rows()).to_yaml();
        let parsed: KubeList<Row> = serde_yaml_ng::from_str(&yaml).expect("valid YAML");
        assert_eq!(parsed.api_version, "v1");
        assert_eq!(parsed.kind, "List");
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[0].name, "alpha");
        assert_eq!(parsed.items[1].count, 2);
    }

    #[test]
    fn to_table_prints_a_header_row_then_one_line_per_item() {
        let table = KubeList::new(rows()).to_table();
        let lines: Vec<&str> = table.lines().collect();
        // The NOTHING preset draws no borders: header + one line per item.
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("NAME"));
        assert!(lines[0].contains("COUNT"));
        assert!(lines[1].contains("alpha"));
        assert!(lines[1].contains('1'));
        assert!(lines[2].contains("beta"));
        assert!(lines[2].contains('2'));
    }

    #[test]
    fn an_empty_list_still_renders_the_headers() {
        let list: KubeList<Row> = KubeList::new(vec![]);
        let table = list.to_table();
        assert!(table.contains("NAME"));
        assert!(table.contains("COUNT"));
        // Header only — no data rows.
        assert_eq!(table.lines().count(), 1);
        // And the other two formats keep the envelope with an empty item list.
        assert!(list.to_json().contains("\"items\": []"));
        assert!(list.to_yaml().contains("items: []"));
    }

    #[test]
    fn to_output_dispatches_on_the_selected_format() {
        let list = KubeList::new(rows());
        assert_eq!(list.to_output(ContextFormat::Json), list.to_json());
        assert_eq!(list.to_output(ContextFormat::Yaml), list.to_yaml());
        assert_eq!(list.to_output(ContextFormat::Table), list.to_table());
        // Default is the table rendering.
        assert_eq!(list.to_output(ContextFormat::default()), list.to_table());
    }
}
