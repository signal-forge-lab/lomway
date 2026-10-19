//! A deliberately small JSON Schema subset for action and pipeline
//! parameters. Unsupported keywords are rejected when the definition file is
//! loaded, so a definition never silently validates less than it says.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const MAX_STRING_CHARS: usize = 4096;
const MAX_ARRAY_ITEMS: usize = 64;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObjectSchema {
    #[serde(rename = "type", default = "object_type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) properties: BTreeMap<String, PropertySchema>,
    #[serde(default)]
    pub(crate) required: Vec<String>,
    /// Only `false` is supported (the default).
    #[serde(rename = "additionalProperties", default)]
    pub(crate) additional_properties: bool,
}

impl Default for ObjectSchema {
    fn default() -> Self {
        Self {
            kind: object_type(),
            properties: BTreeMap::new(),
            required: Vec::new(),
            additional_properties: false,
        }
    }
}

fn object_type() -> String {
    "object".to_string()
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PropertySchema {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(rename = "enum", default, skip_serializing_if = "Option::is_none")]
    pub(crate) enumeration: Option<Vec<Value>>,
    #[serde(rename = "minLength", default, skip_serializing_if = "Option::is_none")]
    pub(crate) min_length: Option<usize>,
    #[serde(rename = "maxLength", default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) maximum: Option<f64>,
    /// `relative-path`: a safe path relative to the action working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) format: Option<String>,
    /// For `array`: item type must be `string`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) items: Option<Box<PropertySchema>>,
    #[serde(rename = "maxItems", default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_items: Option<usize>,
    /// Values starting with `-` are rejected unless this is true, so a
    /// parameter cannot smuggle an option into a command line.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) allow_leading_dash: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) default: Option<Value>,
}

impl ObjectSchema {
    pub(crate) fn check_definition(&self, owner: &str) -> Result<()> {
        ensure!(
            self.kind == "object",
            "{owner}: parameters.type must be \"object\""
        );
        ensure!(
            !self.additional_properties,
            "{owner}: parameters.additionalProperties must be false"
        );
        for name in &self.required {
            ensure!(
                self.properties.contains_key(name),
                "{owner}: required parameter '{name}' has no property definition"
            );
        }
        for (name, property) in &self.properties {
            ensure!(
                is_identifier(name),
                "{owner}: parameter name '{name}' must match [A-Za-z_][A-Za-z0-9_]*"
            );
            property.check_definition(&format!("{owner}.{name}"), false)?;
        }
        Ok(())
    }

    /// Validate `input` and return the normalized parameter map with
    /// defaults applied.
    pub(crate) fn validate(
        &self,
        input: Option<&Map<String, Value>>,
    ) -> Result<Map<String, Value>> {
        let empty = Map::new();
        let input = input.unwrap_or(&empty);
        for key in input.keys() {
            ensure!(
                self.properties.contains_key(key),
                "unknown parameter '{key}'"
            );
        }
        let mut out = Map::new();
        for (name, property) in &self.properties {
            match input.get(name).filter(|value| !value.is_null()) {
                Some(value) => {
                    property.validate(name, value)?;
                    out.insert(name.clone(), value.clone());
                }
                None => {
                    if let Some(default) = &property.default {
                        out.insert(name.clone(), default.clone());
                    } else if self.required.contains(name) {
                        bail!("missing required parameter '{name}'");
                    }
                }
            }
        }
        Ok(out)
    }

    /// JSON form for list/describe output.
    pub(crate) fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

impl PropertySchema {
    fn check_definition(&self, owner: &str, nested: bool) -> Result<()> {
        match self.kind.as_str() {
            "string" | "integer" | "number" | "boolean" => {}
            "array" if !nested => {
                let items = self
                    .items
                    .as_ref()
                    .ok_or_else(|| anyhow!("{owner}: array parameters need `items`"))?;
                ensure!(
                    items.kind == "string",
                    "{owner}: array items must be strings"
                );
                items.check_definition(&format!("{owner}[]"), true)?;
            }
            other => bail!("{owner}: unsupported parameter type '{other}'"),
        }
        if let Some(format) = &self.format {
            ensure!(
                format == "relative-path" && self.kind == "string",
                "{owner}: only format = \"relative-path\" on strings is supported"
            );
        }
        if let Some(values) = &self.enumeration {
            ensure!(!values.is_empty(), "{owner}: enum must not be empty");
            for value in values {
                self.validate_scalar(owner, value)?;
            }
        }
        if let Some(default) = &self.default {
            self.validate(owner, default)?;
        }
        Ok(())
    }

    pub(crate) fn validate(&self, name: &str, value: &Value) -> Result<()> {
        if self.kind == "array" {
            let items = value
                .as_array()
                .ok_or_else(|| anyhow!("parameter '{name}' must be an array"))?;
            let max = self
                .max_items
                .unwrap_or(MAX_ARRAY_ITEMS)
                .min(MAX_ARRAY_ITEMS);
            ensure!(items.len() <= max, "parameter '{name}' has too many items");
            let item_schema = self
                .items
                .as_deref()
                .ok_or_else(|| anyhow!("parameter '{name}' has no item schema"))?;
            for item in items {
                item_schema.validate_scalar(name, item)?;
            }
            return Ok(());
        }
        self.validate_scalar(name, value)
    }

    fn validate_scalar(&self, name: &str, value: &Value) -> Result<()> {
        match self.kind.as_str() {
            "string" => {
                let text = value
                    .as_str()
                    .ok_or_else(|| anyhow!("parameter '{name}' must be a string"))?;
                let chars = text.chars().count();
                ensure!(chars <= MAX_STRING_CHARS, "parameter '{name}' is too long");
                ensure!(
                    !text.chars().any(char::is_control),
                    "parameter '{name}' must not contain control characters"
                );
                if let Some(min) = self.min_length {
                    ensure!(chars >= min, "parameter '{name}' is shorter than {min}");
                }
                if let Some(max) = self.max_length {
                    ensure!(chars <= max, "parameter '{name}' is longer than {max}");
                }
                ensure!(
                    self.allow_leading_dash || !text.starts_with('-'),
                    "parameter '{name}' must not start with '-'"
                );
                if self.format.as_deref() == Some("relative-path") {
                    super::paths::SafeRelativePath::parse(text)
                        .map_err(|error| anyhow!("parameter '{name}': {error}"))?;
                }
            }
            "integer" => {
                let number = value
                    .as_i64()
                    .ok_or_else(|| anyhow!("parameter '{name}' must be an integer"))?;
                self.check_range(name, number as f64)?;
            }
            "number" => {
                let number = value
                    .as_f64()
                    .ok_or_else(|| anyhow!("parameter '{name}' must be a number"))?;
                self.check_range(name, number)?;
            }
            "boolean" => {
                ensure!(value.is_boolean(), "parameter '{name}' must be a boolean");
            }
            other => bail!("parameter '{name}' has unsupported type '{other}'"),
        }
        if let Some(values) = &self.enumeration {
            ensure!(
                values.contains(value),
                "parameter '{name}' must be one of {}",
                Value::Array(values.clone())
            );
        }
        Ok(())
    }

    fn check_range(&self, name: &str, number: f64) -> Result<()> {
        if let Some(min) = self.minimum {
            ensure!(number >= min, "parameter '{name}' is below {min}");
        }
        if let Some(max) = self.maximum {
            ensure!(number <= max, "parameter '{name}' is above {max}");
        }
        Ok(())
    }
}

pub(crate) fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 64
        && chars
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// Render a validated scalar as one command-line argument.
pub(crate) fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema(text: &str) -> ObjectSchema {
        let schema: ObjectSchema = toml::from_str(text).unwrap();
        schema.check_definition("t").unwrap();
        schema
    }

    #[test]
    fn validates_types_ranges_enums_and_unknown_keys() {
        let schema = schema(
            r#"
type = "object"
required = ["name"]
[properties.name]
type = "string"
maxLength = 5
[properties.level]
type = "integer"
minimum = 1
maximum = 3
default = 2
[properties.mode]
type = "string"
enum = ["a", "b"]
[properties.files]
type = "array"
items = { type = "string", format = "relative-path" }
"#,
        );
        let ok = schema
            .validate(Some(
                json!({"name": "abc", "files": ["x/y"]})
                    .as_object()
                    .unwrap(),
            ))
            .unwrap();
        assert_eq!(ok["level"], json!(2));
        for bad in [
            json!({}),
            json!({"name": "toolong"}),
            json!({"name": 1}),
            json!({"name": "a", "level": 9}),
            json!({"name": "a", "mode": "c"}),
            json!({"name": "a", "other": 1}),
            json!({"name": "-rf"}),
            json!({"name": "a\nb"}),
            json!({"name": "a", "files": ["../x"]}),
            json!({"name": "a", "files": "x"}),
        ] {
            assert!(
                schema.validate(Some(bad.as_object().unwrap())).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn unsupported_definitions_are_refused() {
        for text in [
            "type = \"array\"",
            "additionalProperties = true",
            "required = [\"x\"]",
            "[properties.x]\ntype = \"object\"",
            "[properties.x]\ntype = \"string\"\nformat = \"uri\"",
            "[properties.x]\ntype = \"array\"",
            "[properties.\"bad-name\"]\ntype = \"string\"",
            "[properties.x]\ntype = \"string\"\npattern = \"x\"",
        ] {
            let parsed: Result<ObjectSchema, _> = toml::from_str(text);
            if let Ok(schema) = parsed {
                assert!(schema.check_definition("t").is_err(), "{text}");
            }
        }
    }
}
