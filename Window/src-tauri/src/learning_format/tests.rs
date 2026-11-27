use super::*;
use serde_json::json;

// Handwritten synthetic data only.
fn learning(version: &str) -> Value {
    json!({"schema_version":version,"dataset_id":"fixture-a","title":"Synthetic","items":[{"id":"one","question":"Synthetic question","answer":"a  b"}]})
}
fn v2() -> Value {
    let mut v = learning("2.0");
    v["items"][0]["type"] = json!("short_answer");
    v
}
fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}
fn descriptive() -> Value {
    json!({"id":"two","question":"Synthetic description","type":"descriptive","reference_answer":"one two three four five\r\nsix","key_points":["first\nsecond"]})
}

#[test]
fn legacy_conversion_retains_exact_strings_and_optional_unknown_metadata() {
    let mut v = learning("1.0");
    v["custom"] = json!({"note":true});
    v["items"][0]["custom"] = json!(42);
    let parsed = parse_learning(&bytes(&v)).unwrap();
    assert_eq!(
        parsed.items[0].criteria(),
        Criteria::ShortAnswer {
            answer: "a  b".into(),
            accepted_answers: vec![]
        }
    );
    let exported = parse_learning(&export_learning_v2(&parsed).unwrap()).unwrap();
    assert_eq!(exported.schema_version, "2.0");
    assert_eq!(exported.items, parsed.items);
    for key in ["type", "reference_answer", "key_points"] {
        let mut bad = v.clone();
        bad["items"][0][key] = Value::Null;
        assert!(parse_learning(&bytes(&bad)).is_err());
    }
}

#[test]
fn mixed_v2_roundtrip_preserves_descriptive_criteria_and_short_answer_limits() {
    let mut v = v2();
    v["items"].as_array_mut().unwrap().push(descriptive());
    let parsed = parse_learning(&bytes(&v)).unwrap();
    assert_eq!(
        parsed.items[1].criteria(),
        Criteria::Descriptive {
            reference_answer: "one two three four five\r\nsix".into(),
            key_points: vec!["first\nsecond".into()]
        }
    );
    let roundtrip = parse_learning(&export_learning_v2(&parsed).unwrap()).unwrap();
    assert_eq!(parsed.items, roundtrip.items);
    for (text, ok) in [
        ("one two three four", true),
        ("one two three four five", false),
        ("a\nb", false),
        (" a", false),
        ("\u{0085}", false),
        ("\u{feff}", true),
    ] {
        let mut sample = v2();
        sample["items"][0]["answer"] = json!(text);
        assert_eq!(parse_learning(&bytes(&sample)).is_ok(), ok);
        sample = v2();
        sample["items"][0]["accepted_answers"] = json!([text]);
        assert_eq!(parse_learning(&bytes(&sample)).is_ok(), ok);
    }
}

#[test]
fn unknown_and_mixed_fields_types_versions_and_duplicates_are_rejected() {
    for variant in 0..8 {
        let mut v = v2();
        match variant {
            0 => v["schema_version"] = json!("3.0"),
            1 => v["items"][0]["type"] = json!("other"),
            2 => v["items"][0]["reference_answer"] = json!("extra"),
            3 => v["other"] = json!(true),
            4 => v["items"][0]["other"] = json!(true),
            5 => {
                v["items"][0] = descriptive();
                v["items"][0]["answer"] = json!("bad");
            }
            6 => v["items"][0]["id"] = json!("Bad_ID"),
            _ => {
                let item = v["items"][0].clone();
                v["items"].as_array_mut().unwrap().push(item);
            }
        }
        assert!(parse_learning(&bytes(&v)).is_err());
    }
    let repeated=br#"{"schema_version":"2.0","schema_version":"2.0","dataset_id":"x","title":"x","items":[]}"#;
    assert!(parse_learning(repeated).is_err());
}

#[test]
fn invalid_unicode_empty_criteria_and_resource_boundaries_are_rejected() {
    assert!(parse_learning(&[0xff]).is_err());
    assert!(parse_learning(br#"{"schema_version":"2.0","title":"\uD800"}"#).is_err());
    for value in ["", "\u{0085}\u{3000}"] {
        let mut v = v2();
        v["items"][0] = descriptive();
        v["items"][0]["reference_answer"] = json!(value);
        assert!(parse_learning(&bytes(&v)).is_err());
    }
    let mut v = v2();
    v["items"][0] = descriptive();
    v["items"][0]["key_points"] = json!([]);
    assert!(parse_learning(&bytes(&v)).is_err());
    v["items"][0]["key_points"] = json!(["\u{00a0}"]);
    assert!(parse_learning(&bytes(&v)).is_err());
    for size in [65_536, 65_537] {
        v["items"][0] = descriptive();
        v["items"][0]["reference_answer"] = json!("x".repeat(size));
        assert_eq!(parse_learning(&bytes(&v)).is_ok(), size == 65_536);
    }
    assert!(parse_learning(&vec![b' '; MAX_JSON_BYTES + 1]).is_err());
    v = v2();
    v["items"][0]["question"] = json!("x".repeat(16_385));
    assert!(parse_learning(&bytes(&v)).is_err());
    v = v2();
    v["items"][0]["accepted_answers"] = json!(vec!["x"; 65]);
    assert!(parse_learning(&bytes(&v)).is_err());
}

#[test]
fn supplements_keep_free_payload_and_source_refs_but_validate_link_shape() {
    let data = v2();
    let mut s = json!({"schema_version":"2.0","dataset_id":"fixture-a","dataset_metadata":{"custom":[1]},"items":[{"id":"one","custom":{"type":"free"},"source_refs":[{"dataset_id":"Original ID","item_id":"Original_ID"}]}],"learning_links":[{"item_id":"one","prerequisites":[{"dataset_id":"missing","item_id":"two"}],"stage":"foundation"}]});
    let parsed = parse_package(&bytes(&data), Some(&bytes(&s))).unwrap();
    assert_eq!(parsed.links.len(), 1);
    assert!(parsed.supplement.unwrap().items[0].get("custom").is_some());
    s["learning_links"][0]["prerequisites"][0]["item_id"] = json!("Bad_ID");
    assert!(parse_package(&bytes(&data), Some(&bytes(&s))).is_err());
    s["learning_links"] = json!([]);
    s["schema_version"] = json!("1.0");
    assert!(parse_package(&bytes(&data), Some(&bytes(&s))).is_err());
}

fn with_links(id: &str, targets: &[(&str, &str)]) -> ParsedPackage {
    let mut v = v2();
    v["dataset_id"] = json!(id);
    let s = json!({"schema_version":"2.0","dataset_id":id,"items":[],"learning_links":[{"item_id":"one","prerequisites":targets.iter().map(|(d,i)|json!({"dataset_id":d,"item_id":i})).collect::<Vec<_>>()}]});
    parse_package(&bytes(&v), Some(&bytes(&s))).unwrap()
}

#[test]
fn link_diagnostics_find_cycles_without_rejecting_learning_and_recheck_installation() {
    let packages = vec![
        with_links(
            "a",
            &[("b", "one"), ("a", "one"), ("b", "one"), ("missing", "one")],
        ),
        with_links("b", &[("a", "one"), ("c", "one")]),
        with_links("c", &[]),
    ];
    let links = resolve_links(&packages);
    assert_eq!(
        links.iter().map(|e| e.issue.clone()).collect::<Vec<_>>(),
        vec![
            Some(LinkIssue::Cycle),
            Some(LinkIssue::SelfReference),
            Some(LinkIssue::Duplicate),
            Some(LinkIssue::Missing),
            Some(LinkIssue::Cycle),
            None
        ]
    );
    let mut partial = vec![with_links("a", &[("b", "one")])];
    assert_eq!(resolve_links(&partial)[0].issue, Some(LinkIssue::Missing));
    partial.push(with_links("b", &[]));
    assert_eq!(resolve_links(&partial)[0].issue, None);
}

#[test]
fn catalog_accepts_v1_and_v2_and_rejects_path_escape_count_mismatch_and_v1_mixing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("old.json"), bytes(&learning("1.0"))).unwrap();
    let mut newer = v2();
    newer["dataset_id"] = json!("fixture-b");
    std::fs::write(root.join("new.json"), bytes(&newer)).unwrap();
    let mut catalog = json!({"schema_version":"2.0","datasets":[{"dataset_id":"fixture-a","title":"Synthetic","learning_file":"old.json","question_count":1},{"dataset_id":"fixture-b","title":"Synthetic","learning_file":"new.json","question_count":1}]});
    let run = |v: &Value| {
        std::fs::write(root.join("catalog.json"), bytes(v)).unwrap();
        load_catalog(root)
    };
    assert_eq!(run(&catalog).unwrap().len(), 2);
    catalog["schema_version"] = json!("1.0");
    assert!(run(&catalog).is_err());
    catalog["schema_version"] = json!("2.0");
    catalog["datasets"][1]["question_count"] = json!(2);
    assert!(run(&catalog).is_err());
    catalog["datasets"][1]["question_count"] = json!(1);
    for path in ["../outside.json", "/absolute.json", "C:\\outside.json"] {
        catalog["datasets"][0]["learning_file"] = json!(path);
        assert!(run(&catalog).is_err());
    }
}

#[test]
fn errors_do_not_echo_input_values() {
    let mut v = v2();
    let secret = "synthetic-sensitive-marker";
    v["items"][0]["type"] = json!(secret);
    let err = parse_learning(&bytes(&v)).err().unwrap();
    let encoded = serde_json::to_string(&err).unwrap();
    assert!(!encoded.contains(secret));
}

#[test]
fn legacy_catalog_free_fields_are_compatible_but_v2_fields_are_strict() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("learning.json"), bytes(&learning("1.0"))).unwrap();
    let mut catalog = json!({"schema_version":"1.0","note":"old metadata","datasets":[{"dataset_id":"fixture-a","title":"Synthetic","learning_file":"learning.json","question_count":1,"note":"old entry metadata"}]});
    std::fs::write(dir.path().join("catalog.json"), bytes(&catalog)).unwrap();
    assert!(load_catalog(dir.path()).is_ok());
    catalog["schema_version"] = json!("2.0");
    std::fs::write(dir.path().join("catalog.json"), bytes(&catalog)).unwrap();
    assert!(load_catalog(dir.path()).is_err());
}

#[test]
fn deep_link_graph_is_iterative_and_does_not_mark_acyclic_descendants_as_cycles() {
    let count = 4000;
    let items = (0..count)
        .map(|i| Item::ShortAnswer {
            id: format!("i-{i}"),
            question: "Synthetic".into(),
            answer: "x".into(),
            accepted_answers: vec![],
        })
        .collect();
    let links = (0..count - 1)
        .map(|i| LearningLink {
            item_id: format!("i-{i}"),
            prerequisites: vec![ItemKey {
                dataset_id: "a".into(),
                item_id: format!("i-{}", i + 1),
            }],
            stage: None,
        })
        .collect();
    let package = ParsedPackage {
        learning: Learning {
            schema_version: "2.0".into(),
            dataset_id: "a".into(),
            title: "Synthetic".into(),
            items,
        },
        supplement: None,
        links,
    };
    let edges = resolve_links(&[package]);
    assert_eq!(edges.len(), count - 1);
    assert!(edges.iter().all(|e| e.issue.is_none()));
}
