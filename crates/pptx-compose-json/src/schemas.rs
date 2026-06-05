use serde_json::Value;

use crate::agent_view::AgentView;
use crate::schema_versions::AGENT_VIEW_SCHEMA;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonError {
    SerializeSchema(String),
}

pub fn agent_view_json_schema() -> Result<Value, JsonError> {
    let schema = schemars::schema_for!(AgentView);
    let mut value =
        serde_json::to_value(schema).map_err(|err| JsonError::SerializeSchema(err.to_string()))?;

    if let Some(object) = value.as_object_mut() {
        object.insert(
            "$id".to_owned(),
            Value::String(AGENT_VIEW_SCHEMA.to_owned()),
        );
    }

    Ok(value)
}
