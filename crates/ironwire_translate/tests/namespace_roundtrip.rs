use ironwire_core::protocol::Protocol;
use ironwire_translate::{emit_request, parse_request};
use serde_json::json;

#[test]
fn namespace_functions_reach_chat_instead_of_being_dropped() {
    let request = json!({"input":"hello", "tools":[{"type":"namespace", "name":"multi_agent_v1", "tools":[{"type":"function", "name":"spawn_agent", "parameters":{"type":"object"}}]}]});
    let (request, _) = ironwire_translate::namespaces::NamespaceMap::flatten(&request).unwrap();
    let parsed = parse_request(Protocol::OpenAiResponses, &request);
    let (chat, dropped) = emit_request(Protocol::OpenAiChat, &parsed, "model");
    assert!(dropped.tools.is_empty(), "{:?}", dropped.tools);
    assert_eq!(chat["tools"].as_array().unwrap().len(), 1);
}

#[test]
fn namespace_history_choice_and_answers_keep_the_original_identity() {
    use ironwire_translate::namespaces::NamespaceMap;
    let source = json!({"tools":[
        {"type":"function","name":"iw_namespace_0","parameters":{}},
        {"type":"namespace","name":"one","description":"First namespace","tools":[{"type":"function","name":"same","strict":true,"parameters":{"type":"object"}}]},
        {"type":"namespace","name":"two","tools":[{"type":"function","name":"same","parameters":{}}]}
    ], "input":[{"type":"function_call","call_id":"call_1","namespace":"two","name":"same","arguments":"{}"}, {"type":"function_call_output","call_id":"call_1","output":"OK"}],
    "tool_choice":{"type":"function","namespace":"one","name":"same"}});
    let (flat, names) = NamespaceMap::flatten(&source).unwrap();
    assert_eq!(flat["tools"][0]["name"], "iw_namespace_0");
    assert_eq!(flat["tools"][1]["name"], "iw_namespace_1");
    assert_eq!(flat["tools"][1]["strict"], true);
    assert_eq!(flat["tools"][2]["name"], "iw_namespace_2");
    assert_eq!(flat["input"][0]["name"], "iw_namespace_2");
    assert!(flat["input"][0].get("namespace").is_none());
    assert_eq!(flat["input"][1], source["input"][1]);
    assert_eq!(flat["tool_choice"]["name"], "iw_namespace_1");
    let mut response = json!({"output":[{"type":"function_call","call_id":"call_2","name":"iw_namespace_2","arguments":"{}"}, {"type":"function_call","name":"iw_namespace_0","arguments":"{}"}]});
    names.restore_response(&mut response);
    assert_eq!(response["output"][0]["namespace"], "two");
    assert_eq!(response["output"][0]["name"], "same");
    assert_eq!(response["output"][1]["name"], "iw_namespace_0");
    assert!(response["output"][1].get("namespace").is_none());
    // Rebuilding on the next turn uses the declaration map, not a process-wide
    // mutable name table. Long namespace names never reach Chat's name limit.
    let mut next = source.clone();
    next["input"][0] = response["output"][0].clone();
    let (flat_next, _) = NamespaceMap::flatten(&next).unwrap();
    assert_eq!(flat_next["input"][0]["name"], "iw_namespace_2");
}

#[test]
fn streamed_namespace_calls_restore_identity_after_fragmented_arguments() {
    use ironwire_translate::{Translator, namespaces::NamespaceMap};
    let (_, names) = NamespaceMap::flatten(&json!({"tools":[{"type":"namespace","name":"multi_agent_v1","tools":[{"type":"function","name":"close_agent","parameters":{}}]}]})).unwrap();
    let mut translator = Translator::new(Protocol::OpenAiChat, Protocol::OpenAiResponses, "model")
        .with_namespaces(names);
    let events = [
        json!({"id":"chat_1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"iw_namespace_0","arguments":"{\"target\":"}}]},"finish_reason":null}]}),
        json!({"id":"chat_1","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"test\"}"}}]},"finish_reason":"tool_calls"}]}),
    ];
    let mut bytes = Vec::new();
    for event in events {
        bytes.extend(translator.push(format!("data: {event}\n\n").as_bytes()));
    }
    bytes.extend(translator.finish());
    let text = String::from_utf8(bytes).unwrap();
    let items: Vec<_> = text
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .filter(|v| v["type"] == "response.output_item.done")
        .collect();
    assert_eq!(items.len(), 1, "{text}");
    assert_eq!(items[0]["item"]["namespace"], "multi_agent_v1");
    assert_eq!(items[0]["item"]["name"], "close_agent");
    assert_eq!(items[0]["item"]["arguments"], "{\"target\":\"test\"}");
}

#[test]
fn namespace_translation_refuses_ambiguous_or_unrepresentable_tools() {
    use ironwire_translate::namespaces::NamespaceMap;
    for children in [
        json!([{"type":"custom","name":"patch"}]),
        json!([{"type":"function","name":"same"},{"type":"function","name":"same"}]),
    ] {
        assert!(
            NamespaceMap::flatten(
                &json!({"tools":[{"type":"namespace","name":"n","tools":children}]})
            )
            .is_err()
        );
    }
    assert!(NamespaceMap::flatten(&json!({"input":[{"type":"function_call","namespace":"missing","name":"x","arguments":"{}"}]})).is_err());
}

#[test]
fn translated_text_has_a_complete_responses_item_lifecycle() {
    let mut translator = ironwire_translate::Translator::new(
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        "model",
    );
    let source = concat!(
        "data: {\"id\":\"chat_text\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"chat_text\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" world\"},\"finish_reason\":\"stop\"}]}\n\n"
    );
    let mut bytes = translator.push(source.as_bytes());
    bytes.extend(translator.finish());
    let text = String::from_utf8(bytes).unwrap();
    let events: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect();
    let added = events
        .iter()
        .position(|e| e["type"] == "response.output_item.added")
        .expect("message item starts before text");
    let delta = events
        .iter()
        .position(|e| e["type"] == "response.output_text.delta")
        .unwrap();
    assert!(added < delta);
    let done = events
        .iter()
        .find(|e| e["type"] == "response.output_item.done")
        .unwrap();
    assert_eq!(done["item"]["content"][0]["text"], "Hello world");
    let completed = events
        .iter()
        .find(|e| e["type"] == "response.completed")
        .unwrap();
    assert_eq!(completed["response"]["output"][0], done["item"]);
    assert_eq!(events[delta]["item_id"], done["item"]["id"]);
    assert_eq!(events[delta]["content_index"], 0);
}

#[test]
fn retained_responses_text_is_bounded_and_overflow_cannot_look_successful() {
    let mut translator = ironwire_translate::Translator::new(
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        "model",
    );
    let frame = format!(
        "data: {}\n\n",
        json!({"id":"c", "choices":[{"index":0,"delta":{"content":"x".repeat(65536)},"finish_reason":null}]})
    );
    let mut output = Vec::new();
    for _ in 0..66 {
        output.extend(translator.push(frame.as_bytes()));
    }
    output.extend(translator.finish());
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("translation_output_limit"));
    assert!(!text.contains("response.completed"));
}

#[test]
fn flat_historical_function_names_cannot_collide_with_namespace_aliases() {
    use ironwire_translate::namespaces::NamespaceMap;
    let (flat, _) = NamespaceMap::flatten(&json!({"tools":[{"type":"namespace","name":"n","tools":[{"type":"function","name":"f"}]}],"input":[{"type":"function_call","name":"iw_namespace_0","call_id":"old","arguments":"{}"}]})).unwrap();
    assert_ne!(flat["tools"][0]["name"], flat["input"][0]["name"]);
}
