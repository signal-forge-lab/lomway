//! Portable input schemas for Lomway-owned tools.
//!
//! `schemars` emits draft 2020-12 idioms that some MCP clients cannot turn
//! into function declarations. Gemini's function-calling schema (an OpenAPI
//! 3.0 subset) is the strictest known consumer: it rejects a `type` array
//! such as `["string", "null"]` (`Proto field is not repeating, cannot start
//! list`) and Rust-specific formats such as `uint`. This module rewrites a
//! generated schema into the common subset every client accepts.
//!
//! Only the published schema changes. Handlers still deserialize into the
//! same Rust types, so `null` for an optional field and an omitted field
//! keep behaving identically for every caller.

use serde_json::{Map, Value};
use tower_mcp::schemars::{JsonSchema, schema_for};

/// Integer formats every function-calling schema dialect understands.
const PORTABLE_INTEGER_FORMATS: [&str; 2] = ["int32", "int64"];
/// Number formats every function-calling schema dialect understands.
const PORTABLE_NUMBER_FORMATS: [&str; 2] = ["float", "double"];

/// Generate the published input schema for `T` in the portable subset.
pub(super) fn portable_input_schema<T: JsonSchema>() -> Value {
    let mut schema = serde_json::to_value(schema_for!(T))
        .unwrap_or_else(|_| serde_json::json!({ "type": "object" }));
    inline_local_refs(&mut schema);
    portable_schema(schema)
}

/// Deepest chain of nested local references that is inlined. A deeper (or
/// recursive) chain keeps its remaining `$ref`s and the definitions block.
const MAX_REF_DEPTH: usize = 16;

/// Replace `{"$ref": "#/$defs/X", ...}` with the definition of `X` (sibling
/// keywords such as `description` win), so clients that do not resolve
/// references (Gemini's OpenAPI subset) still see the nested fields. The
/// `$defs`/`definitions` block is dropped once nothing refers to it.
fn inline_local_refs(schema: &mut Value) {
    let Value::Object(root) = schema else {
        return;
    };
    let mut defs = Map::new();
    for keyword in ["$defs", "definitions"] {
        if let Some(Value::Object(map)) = root.get(keyword) {
            for (name, def) in map {
                defs.insert(format!("#/{keyword}/{name}"), def.clone());
            }
        }
    }
    if defs.is_empty() {
        return;
    }
    let mut remaining = false;
    for (key, child) in root.iter_mut() {
        if key != "$defs" && key != "definitions" {
            inline_value(child, &defs, 0, &mut remaining);
        }
    }
    if !remaining {
        root.remove("$defs");
        root.remove("definitions");
    }
}

fn inline_value(value: &mut Value, defs: &Map<String, Value>, depth: usize, remaining: &mut bool) {
    match value {
        Value::Object(object) => {
            let target = object
                .get("$ref")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if let Some(target) = target
                && let Some(Value::Object(def)) = defs.get(&target)
            {
                if depth >= MAX_REF_DEPTH {
                    *remaining = true;
                    return;
                }
                object.remove("$ref");
                for (key, child) in def {
                    object.entry(key.clone()).or_insert_with(|| child.clone());
                }
                for child in object.values_mut() {
                    inline_value(child, defs, depth + 1, remaining);
                }
                return;
            }
            if object.contains_key("$ref") {
                *remaining = true;
            }
            for child in object.values_mut() {
                inline_value(child, defs, depth, remaining);
            }
        }
        Value::Array(items) => {
            for child in items {
                inline_value(child, defs, depth, remaining);
            }
        }
        _ => {}
    }
}

/// Rewrite one JSON schema into the portable subset.
///
/// * drops every `$schema` declaration;
/// * turns `"type": [T, "null"]` into `"type": T` (nullability is expressed
///   by leaving the property out of `required`), and a remaining multi-type
///   array into `anyOf` of single types;
/// * collapses `anyOf`/`oneOf` of exactly one schema plus `{"type":"null"}`
///   into that schema;
/// * removes non-portable `format` values (for example `uint`, `uint8`,
///   `int16`), keeping any `minimum` bound that schemars already emitted.
pub(super) fn portable_schema(mut schema: Value) -> Value {
    sanitize(&mut schema);
    schema
}

/// Keywords whose value is a map from names to subschemas.
const SCHEMA_MAP_KEYWORDS: [&str; 5] = [
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
];
/// Keywords whose value is an array of subschemas.
const SCHEMA_ARRAY_KEYWORDS: [&str; 4] = ["anyOf", "oneOf", "allOf", "prefixItems"];
/// Keywords whose value is one subschema (or, for `items`, possibly an
/// array of subschemas in older drafts).
const SCHEMA_KEYWORDS: [&str; 12] = [
    "items",
    "additionalProperties",
    "additionalItems",
    "unevaluatedProperties",
    "unevaluatedItems",
    "propertyNames",
    "contains",
    "not",
    "if",
    "then",
    "else",
    "contentSchema",
];

/// Visit every subschema of `object`: never instance data such as `enum`,
/// `const`, `default` or `examples`, and never property names.
fn for_each_subschema(object: &mut Map<String, Value>, mut visit: impl FnMut(&mut Value)) {
    for (key, child) in object.iter_mut() {
        let key = key.as_str();
        if SCHEMA_MAP_KEYWORDS.contains(&key) {
            if let Value::Object(map) = child {
                map.values_mut().for_each(&mut visit);
            }
        } else if SCHEMA_ARRAY_KEYWORDS.contains(&key) {
            if let Value::Array(items) = child {
                items.iter_mut().for_each(&mut visit);
            }
        } else if SCHEMA_KEYWORDS.contains(&key) {
            match child {
                Value::Array(items) => items.iter_mut().for_each(&mut visit),
                other => visit(other),
            }
        }
    }
}

fn sanitize(value: &mut Value) {
    if let Value::Object(object) = value {
        object.remove("$schema");
        collapse_nullable_union(object, "anyOf");
        collapse_nullable_union(object, "oneOf");
        normalize_type(object);
        drop_non_portable_format(object);
        drop_null_default(object);
        for_each_subschema(object, sanitize);
    }
}

fn is_null_type(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("null")
        && schema.as_object().is_some_and(|object| object.len() == 1)
}

fn collapse_nullable_union(object: &mut Map<String, Value>, keyword: &str) {
    let Some(Value::Array(variants)) = object.get(keyword) else {
        return;
    };
    if variants.len() != 2 || !variants.iter().any(is_null_type) {
        return;
    }
    let Some(Value::Object(inner)) = variants.iter().find(|v| !is_null_type(v)).cloned() else {
        return;
    };
    object.remove(keyword);
    for (key, value) in inner {
        // Outer annotations such as `description` win over the variant's.
        object.entry(key).or_insert(value);
    }
}

fn normalize_type(object: &mut Map<String, Value>) {
    let Some(Value::Array(types)) = object.get("type") else {
        return;
    };
    let mut kept: Vec<Value> = types
        .iter()
        .filter(|ty| ty.as_str() != Some("null"))
        .cloned()
        .collect();
    kept.dedup();
    match kept.len() {
        // `["null"]` alone: keep the schema permissive rather than invalid.
        0 => {
            object.remove("type");
        }
        1 => {
            object.insert("type".to_string(), kept.remove(0));
        }
        _ => {
            object.remove("type");
            let variants = kept
                .into_iter()
                .map(|ty| serde_json::json!({ "type": ty }))
                .collect();
            object
                .entry("anyOf".to_string())
                .or_insert(Value::Array(variants));
        }
    }
}

/// `#[serde(default)]` on an `Option` makes schemars emit `"default": null`,
/// which no longer matches the (now single, non-null) `type`. Drop it unless
/// `null` is an explicitly allowed `enum` value.
fn drop_null_default(object: &mut Map<String, Value>) {
    if !object.get("default").is_some_and(Value::is_null) {
        return;
    }
    let null_allowed = object
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().any(Value::is_null));
    if !null_allowed {
        object.remove("default");
    }
}

fn drop_non_portable_format(object: &mut Map<String, Value>) {
    let Some(format) = object.get("format").and_then(Value::as_str) else {
        return;
    };
    let portable = match object.get("type").and_then(Value::as_str) {
        Some("integer") => PORTABLE_INTEGER_FORMATS.contains(&format),
        Some("number") => PORTABLE_NUMBER_FORMATS.contains(&format),
        // String formats (date-time, uri, …) are standard JSON Schema and
        // either honoured or ignored by every client.
        Some("string") => true,
        _ => false,
    };
    if !portable {
        object.remove("format");
    }
}

/// Every place in `schema` that is not in the portable subset, as
/// human-readable JSON pointers. Empty means portable.
#[cfg(test)]
pub(super) fn non_portable_findings(schema: &Value) -> Vec<String> {
    fn walk(value: &mut Value, path: &str, out: &mut Vec<String>) {
        let Value::Object(object) = value else {
            return;
        };
        if object.contains_key("$schema") {
            out.push(format!("{path}/$schema"));
        }
        if object.get("type").is_some_and(Value::is_array) {
            out.push(format!("{path}/type is an array"));
        }
        if let Some(format) = object.get("format").and_then(Value::as_str) {
            let ok = match object.get("type").and_then(Value::as_str) {
                Some("integer") => PORTABLE_INTEGER_FORMATS.contains(&format),
                Some("number") => PORTABLE_NUMBER_FORMATS.contains(&format),
                Some("string") => true,
                _ => false,
            };
            if !ok {
                out.push(format!("{path}/format={format}"));
            }
        }
        for keyword in ["anyOf", "oneOf"] {
            if let Some(Value::Array(variants)) = object.get(keyword)
                && variants.iter().any(is_null_type)
            {
                out.push(format!("{path}/{keyword} with null"));
            }
        }
        let mut index = 0;
        for_each_subschema(object, |child| {
            walk(child, &format!("{path}/{index}"), out);
            index += 1;
        });
    }
    let mut out = Vec::new();
    walk(&mut schema.clone(), "", &mut out);
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[derive(Debug, serde::Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct Sample {
        /// Required text.
        text: String,
        /// Optional text.
        maybe: Option<String>,
        /// Optional count.
        count: Option<usize>,
        /// Small unsigned.
        small: u8,
        /// Optional object.
        extra: Option<serde_json::Map<String, Value>>,
        /// Optional nested struct.
        nested: Option<Nested>,
    }

    #[derive(Debug, serde::Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct Nested {
        /// Optional flag.
        flag: Option<bool>,
    }

    #[test]
    fn generated_schema_is_portable_and_keeps_required_and_bounds() {
        let raw = serde_json::to_value(schema_for!(Sample)).unwrap();
        assert!(
            !non_portable_findings(&raw).is_empty(),
            "fixture should exercise schemars' non-portable output"
        );
        let schema = portable_input_schema::<Sample>();
        assert_eq!(non_portable_findings(&schema), Vec::<String>::new());
        assert_eq!(schema["type"], "object");
        let mut required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        required.sort_unstable();
        assert_eq!(required, ["small", "text"]);
        let properties = &schema["properties"];
        assert_eq!(properties["maybe"]["type"], "string");
        assert_eq!(properties["maybe"]["description"], "Optional text.");
        assert_eq!(properties["count"]["type"], "integer");
        assert_eq!(properties["count"]["minimum"], 0);
        assert!(properties["count"].get("format").is_none());
        assert_eq!(properties["small"]["minimum"], 0);
        assert_eq!(properties["extra"]["type"], "object");
        // The nested struct is reached through a reference or inlined; either
        // way its own optional field must be portable too.
        assert!(!schema.to_string().contains("\"null\""));
    }

    #[test]
    fn rewrites_type_arrays_unions_and_formats() {
        let schema = portable_schema(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "a": {"type": ["integer", "null"], "format": "uint64", "minimum": 0},
                "b": {"type": ["string", "integer"]},
                "c": {"description": "outer", "anyOf": [{"$ref": "#/$defs/X"}, {"type": "null"}]},
                "d": {"type": "integer", "format": "int64"},
                "e": {"type": "number", "format": "double"},
                "f": {"type": "string", "format": "date-time"},
                "g": {"enum": [null, "x"], "default": null},
                "h": {"type": ["null"]},
                "i": {"type": ["string", "null"], "default": null},
                "default": {"type": ["boolean", "null"]}
            },
            "$defs": {"X": {"type": ["object", "null"], "$schema": "nested"}}
        }));
        assert_eq!(non_portable_findings(&schema), Vec::<String>::new());
        let p = &schema["properties"];
        assert_eq!(p["a"], json!({"type": "integer", "minimum": 0}));
        assert_eq!(
            p["b"],
            json!({"anyOf": [{"type": "string"}, {"type": "integer"}]})
        );
        assert_eq!(p["c"], json!({"description": "outer", "$ref": "#/$defs/X"}));
        assert_eq!(p["d"]["format"], "int64");
        assert_eq!(p["e"]["format"], "double");
        assert_eq!(p["f"]["format"], "date-time");
        assert_eq!(p["g"], json!({"enum": [null, "x"], "default": null}));
        assert_eq!(p["h"], json!({}));
        assert_eq!(p["i"], json!({"type": "string"}));
        assert_eq!(p["default"], json!({"type": "boolean"}));
        assert_eq!(schema["$defs"]["X"], json!({"type": "object"}));
    }

    #[test]
    fn local_references_are_inlined_for_generated_schemas() {
        #[derive(Debug, serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct Outer {
            /// The nested value.
            inner: Nested,
            /// Optional nested value.
            maybe: Option<Nested>,
        }
        let schema = portable_input_schema::<Outer>();
        let text = schema.to_string();
        assert!(!text.contains("$ref"), "{text}");
        assert!(!text.contains("$defs"), "{text}");
        assert_eq!(non_portable_findings(&schema), Vec::<String>::new());
        let inner = &schema["properties"]["inner"];
        assert_eq!(inner["type"], "object");
        assert_eq!(inner["description"], "The nested value.");
        assert_eq!(inner["properties"]["flag"]["type"], "boolean");
        assert_eq!(schema["properties"]["maybe"]["type"], "object");
        assert_eq!(schema["required"], json!(["inner"]));
    }

    #[test]
    fn already_portable_schema_is_unchanged() {
        let schema = json!({
            "type": "object",
            "properties": {"name": {"type": "string", "description": "n"}},
            "required": ["name"]
        });
        assert_eq!(portable_schema(schema.clone()), schema);
    }
}
