//! Named, typed skill budget presentations. The source copy module also
//! generates the legacy printf fields; those fields are never translated.

use serde::Deserialize;
use serde_json::{Value, json};

use super::text;
use crate::skill_loop::{SKILL_CONTRACT_SUMMARY, SKILL_MODEL_BUDGET, SKILL_REVIEW_BUDGET};

#[derive(Deserialize)]
struct ContractSummary {
    tasks: i64,
    cases: i64,
    requests: i64,
    output_tokens: i64,
    seconds: i64,
}

#[derive(Deserialize)]
struct ModelBudget {
    owner: String,
    tasks: i64,
    output_tokens: i64,
}

#[derive(Deserialize)]
struct ReviewBudget {
    requests: i64,
    output_tokens: i64,
    owner: String,
}

// Decimal descriptors request native number formatting even when a newer
// source template falls back to English. No precision-losing float conversion.
fn number(value: i64) -> Value {
    json!({"decimal": value.to_string()})
}

pub fn message(operation: &str, input: &Value) -> Option<Value> {
    Some(match operation {
        "skill_learning_contract_summary_line" => {
            let input: ContractSummary = serde_json::from_value(input.clone()).ok()?;
            text(
                SKILL_CONTRACT_SUMMARY.key,
                SKILL_CONTRACT_SUMMARY.english,
                json!({
                    "tasks": number(input.tasks), "cases": number(input.cases),
                    "requests": number(input.requests), "output_tokens": number(input.output_tokens),
                    "seconds": number(input.seconds),
                }),
            )
        }
        "skill_learning_model_budget_line" => {
            let input: ModelBudget = serde_json::from_value(input.clone()).ok()?;
            text(
                SKILL_MODEL_BUDGET.key,
                SKILL_MODEL_BUDGET.english,
                json!({
                    "owner": input.owner, "tasks": number(input.tasks),
                    "output_tokens": number(input.output_tokens),
                }),
            )
        }
        "skill_learning_review_budget_line" => {
            let input: ReviewBudget = serde_json::from_value(input.clone()).ok()?;
            text(
                SKILL_REVIEW_BUDGET.key,
                SKILL_REVIEW_BUDGET.english,
                json!({
                    "requests": number(input.requests), "output_tokens": number(input.output_tokens),
                    "owner": input.owner,
                }),
            )
        }
        _ => return None,
    })
}

pub fn templates() -> Vec<Value> {
    [
        (
            "skill_learning_contract_summary_line",
            json!({"tasks": 1, "cases": 2, "requests": 3, "output_tokens": 4, "seconds": 5}),
        ),
        (
            "skill_learning_model_budget_line",
            json!({"owner": "", "tasks": 1, "output_tokens": 2}),
        ),
        (
            "skill_learning_review_budget_line",
            json!({"requests": 1, "output_tokens": 2, "owner": ""}),
        ),
    ]
    .into_iter()
    .map(|(operation, input)| message(operation, &input).expect("canonical skill parameters"))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_template_is_source_owned_and_carries_only_named_typed_parameters() {
        let messages = templates();
        for (message, source, fields) in [
            (
                &messages[0],
                SKILL_CONTRACT_SUMMARY,
                vec!["cases", "output_tokens", "requests", "seconds", "tasks"],
            ),
            (
                &messages[1],
                SKILL_MODEL_BUDGET,
                vec!["output_tokens", "owner", "tasks"],
            ),
            (
                &messages[2],
                SKILL_REVIEW_BUDGET,
                vec!["output_tokens", "owner", "requests"],
            ),
        ] {
            assert_eq!(message["key"], source.key);
            assert_eq!(message["english"], source.english);
            let mut actual: Vec<_> = message["parameters"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            actual.sort();
            assert_eq!(actual, fields);
        }
    }

    #[test]
    fn owner_is_verbatim_and_integer_values_keep_full_precision() {
        let owner = "model %1$@ {tasks} {owner}\n\"私有\"";
        let message = message(
            "skill_learning_model_budget_line",
            &json!({
                "owner": owner, "tasks": i64::MAX, "output_tokens": -1,
            }),
        )
        .unwrap();
        assert_eq!(message["parameters"]["owner"], owner);
        assert_eq!(
            message["parameters"]["tasks"],
            json!({"decimal": i64::MAX.to_string()})
        );
        assert_eq!(
            message["parameters"]["output_tokens"],
            json!({"decimal": "-1"})
        );
    }

    #[test]
    fn missing_or_wrong_parameter_types_are_refused_not_guessed() {
        for input in [
            json!({}),
            json!({"owner": "x", "tasks": "2", "output_tokens": 3}),
            json!({"owner": 1, "tasks": 2, "output_tokens": 3}),
            json!({"owner": "x", "tasks": 1.5, "output_tokens": 3}),
        ] {
            assert!(message("skill_learning_model_budget_line", &input).is_none());
        }
        assert!(message("unknown_skill_presentation", &json!({})).is_none());
    }
}
