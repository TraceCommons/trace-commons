//! Request-scoped mapping of Responses namespace functions to flat Chat names.
//!
//! Generated names are short and avoid every declared top-level name. The map
//! belongs to one request, so parallel sessions cannot redirect each other's
//! calls. History is mapped from its explicit namespace, never from a guessed
//! dotted name.

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The inverse names needed when a Chat answer returns to a Responses client.
#[derive(Debug, Default, Clone)]
pub struct NamespaceMap {
    names: BTreeMap<String, (String, String)>,
}

impl NamespaceMap {
    /// Flatten namespace functions and rewrite their history and named choice.
    /// Unsupported namespace children are refused, never omitted.
    pub fn flatten(body: &Value) -> Result<(Value, Self), String> {
        let mut body = body.clone();
        let mut map = Self::default();
        let mut occupied: BTreeSet<String> = body
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|t| t["type"] != "namespace")
            .filter_map(|t| t["name"].as_str().map(str::to_owned))
            .collect();
        // Historical flat calls may outlive their tool declaration. They
        // still reserve their name: a new namespace alias must not turn them
        // into a different function on replay.
        if let Some(input) = body.get("input").and_then(Value::as_array) {
            occupied.extend(
                input
                    .iter()
                    .filter(|item| {
                        item["type"] == "function_call"
                            && item.get("namespace").and_then(Value::as_str).is_none()
                    })
                    .filter_map(|item| item["name"].as_str().map(str::to_owned)),
            );
        }
        let mut identities = BTreeSet::new();
        let mut flat = Vec::new();
        if let Some(tools) = body.get("tools").and_then(Value::as_array) {
            for tool in tools {
                if tool["type"] != "namespace" {
                    flat.push(tool.clone());
                    continue;
                }
                let namespace = tool["name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("a tool namespace needs a name")?;
                let children = tool["tools"]
                    .as_array()
                    .ok_or("a tool namespace needs tools")?;
                for child in children {
                    if child["type"] != "function" {
                        return Err(
                            "only function tools in a namespace can translate to Chat".into()
                        );
                    }
                    let name = child["name"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .ok_or("a namespace function needs a name")?;
                    if !identities.insert((namespace.to_owned(), name.to_owned())) {
                        return Err("duplicate namespace function identity".into());
                    }
                    let mut index = map.names.len();
                    let encoded = loop {
                        let candidate = format!("iw_namespace_{index}");
                        if occupied.insert(candidate.clone()) {
                            break candidate;
                        }
                        index += 1;
                    };
                    map.names
                        .insert(encoded.clone(), (namespace.to_owned(), name.to_owned()));
                    let mut function = child.clone();
                    function["name"] = json!(encoded);
                    if let Some(description) =
                        tool["description"].as_str().filter(|s| !s.is_empty())
                    {
                        function["description"] = json!(format!(
                            "{namespace}: {description}\n{}",
                            child["description"].as_str().unwrap_or("")
                        ));
                    }
                    flat.push(function);
                }
            }
            body["tools"] = Value::Array(flat);
        }
        if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
            for item in input {
                if item["type"] == "function_call" {
                    map.flatten_identity(item)?;
                }
            }
        }
        if let Some(choice) = body.get_mut("tool_choice")
            && choice["type"] == "function"
        {
            map.flatten_identity(choice)?;
        }
        Ok((body, map))
    }

    fn flatten_identity(&self, item: &mut Value) -> Result<(), String> {
        let Some(namespace) = item.get("namespace").and_then(Value::as_str) else {
            return Ok(());
        };
        let name = item["name"]
            .as_str()
            .ok_or("a namespaced call needs a name")?;
        let encoded = self
            .names
            .iter()
            .find(|(_, identity)| identity.0 == namespace && identity.1 == name)
            .map(|(encoded, _)| encoded.clone())
            .ok_or("a namespaced call refers to an undeclared function")?;
        item["name"] = json!(encoded);
        item.as_object_mut()
            .ok_or("invalid tool identity")?
            .remove("namespace");
        Ok(())
    }

    /// Restore the original identity in a Responses function-call item.
    pub fn restore_item(&self, item: &mut Value) {
        if item["type"] != "function_call" {
            return;
        }
        if let Some((namespace, name)) = item["name"].as_str().and_then(|n| self.names.get(n)) {
            item["namespace"] = json!(namespace);
            item["name"] = json!(name);
        }
    }

    /// Restore every function-call item in a non-streaming Responses answer.
    pub fn restore_response(&self, body: &mut Value) {
        if let Some(output) = body.get_mut("output").and_then(Value::as_array_mut) {
            for item in output {
                self.restore_item(item);
            }
        }
    }
}
