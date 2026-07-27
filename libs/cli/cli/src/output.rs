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
        serde_yaml::to_string(self).unwrap_or_default()
    }

    /// Render as a borderless text table.
    #[must_use]
    pub fn to_table(&self) -> String {
        let mut table = Table::new();
        table.load_preset(comfy_table::presets::NOTHING);
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
