use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::NativeAddressStandardization;

/// Policy-neutral address evidence asserted into organizational rules.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyDecisionFact {
    pub customer: String,
    pub role: String,
    pub address_valid: bool,
    pub standardized: String,
    pub source_text: String,
    pub reference_status: String,
    pub reference_name: String,
    pub policy_status: String,
    pub policy_reason: String,
}

/// Build the rule fact for a standardized address. Customer identity comes from
/// structured input or reference matching; no demonstration-specific entities
/// are embedded in this shared layer.
#[must_use]
pub fn address_policy_fact(
    input: &Value,
    standardized: &NativeAddressStandardization,
) -> PolicyDecisionFact {
    PolicyDecisionFact {
        customer: standardized
            .components
            .get("customer")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        role: standardized
            .components
            .get("address_role")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| role_from_context(&searchable_text(input)))
            .unwrap_or_else(|| "bill_to".to_string()),
        address_valid: standardized.valid,
        standardized: standardized.display.clone(),
        source_text: searchable_text(input),
        reference_status: "unmatched".to_string(),
        reference_name: String::new(),
        policy_status: "pending".to_string(),
        policy_reason: "awaiting policy evaluation".to_string(),
    }
}

fn searchable_text(input: &Value) -> String {
    match input {
        Value::String(value) => value.clone(),
        Value::Object(values) => values
            .values()
            .map(searchable_text)
            .collect::<Vec<_>>()
            .join(" "),
        Value::Array(values) => values
            .iter()
            .map(searchable_text)
            .collect::<Vec<_>>()
            .join(" "),
        Value::Null => String::new(),
        value => value.to_string(),
    }
}

fn role_from_context(text: &str) -> Option<String> {
    let normalized = text.to_ascii_lowercase();
    if normalized.contains("ship-to")
        || normalized.contains("ship to")
        || normalized.contains("shipto")
    {
        Some("ship_to".to_string())
    } else if normalized.contains("bill-to")
        || normalized.contains("bill to")
        || normalized.contains("billto")
    {
        Some("bill_to".to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::{standardize_structured_address, standardize_unstructured_address};
    use serde_json::json;

    #[test]
    fn unstructured_context_supplies_address_role() {
        let input = json!("Please ship to 111 East Cola Lane, Springfield IL 62701.");
        let standardized = standardize_unstructured_address(input.as_str().unwrap());
        let fact = address_policy_fact(&input, &standardized);
        assert_eq!(fact.role, "ship_to");
        assert_eq!(fact.source_text, input.as_str().unwrap());
    }

    #[test]
    fn structured_components_supply_customer_and_role() {
        let input = json!({
            "customer_name": "King Cola",
            "address": "500 Royal Road",
            "purpose": "bill to"
        });
        let standardized = standardize_structured_address(&input);
        let fact = address_policy_fact(&input, &standardized);
        assert_eq!(fact.customer, "King Cola");
        assert_eq!(fact.role, "bill_to");
    }
}
