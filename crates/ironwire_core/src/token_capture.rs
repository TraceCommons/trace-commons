//! Targeted augmentation for qualified Chat Completions routes only.
/// Add absent probability fields without changing any existing source bytes.
/// `None` means ambiguous JSON or a caller opt-out: forward unchanged and do
/// not claim that detailed capture was requested. Caller fields always win.
#[must_use]
pub fn augment_chat(body: &[u8], top_k: u8) -> Option<Vec<u8>> {
    if top_k > 20 {
        return None;
    }
    let members = crate::admission::members(body).ok()?;
    let field = |name: &str| {
        members
            .iter()
            .find(|(key, _, _)| key == name)
            .map(|(_, start, end)| &body[*start..*end])
    };
    if let Some(existing) = field("logprobs")
        && !serde_json::from_slice::<bool>(existing).ok()?
    {
        return None;
    }
    if let Some(existing) = field("top_logprobs")
        && serde_json::from_slice::<u64>(existing).ok()? > 20
    {
        return None;
    }
    let mut additions = Vec::new();
    if field("logprobs").is_none() {
        additions.push("\"logprobs\":true".to_string());
    }
    if field("top_logprobs").is_none() {
        additions.push(format!("\"top_logprobs\":{top_k}"));
    }
    if additions.is_empty() {
        return Some(body.to_vec());
    }
    let at = body.iter().rposition(|b| *b == b'}')?;
    let fragment = format!(
        "{}{}",
        if members.is_empty() { "" } else { "," },
        additions.join(",")
    );
    let mut output = Vec::with_capacity(body.len() + fragment.len());
    output.extend_from_slice(&body[..at]);
    output.extend_from_slice(fragment.as_bytes());
    output.extend_from_slice(&body[at..]);
    Some(output)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn augmentation_preserves_unrelated_bytes_and_caller_preferences() {
        let input = br#"{ "z":0.100000000000000005, "model":"x", "unknown":"\u00e9" }  "#;
        let output = augment_chat(input, 20).unwrap();
        assert!(output.starts_with(&input[..input.iter().rposition(|b| *b == b'}').unwrap()]));
        let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(parsed["top_logprobs"], 20);
        let explicit = br#"{"logprobs":true,"top_logprobs":5}"#;
        assert_eq!(augment_chat(explicit, 20).unwrap(), explicit);
        assert!(augment_chat(br#"{"logprobs":false}"#, 20).is_none());
        assert!(augment_chat(br#"{"logprobs":true,"logprobs":false}"#, 20).is_none());
    }
}
