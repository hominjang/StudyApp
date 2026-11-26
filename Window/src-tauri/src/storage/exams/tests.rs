use super::*;
use serde_json::json;

#[test]
fn failed_snapshot_creation_does_not_leave_a_partial_exam() {
    let (_temp, _paths, mut store) = setup();
    store.connection.execute_batch("CREATE TEMP TRIGGER fail_snapshot BEFORE INSERT ON exam_responses WHEN NEW.position=1 BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    let mut req = request();
    req.count = Some(3);
    assert!(store.start_exam(req).is_err());
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM exams", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM exam_responses", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
#[ignore = "실제 채점 예시는 Git 제외. 로컬 자료 환경에서만 실행"]
fn private_local_grading_examples() {
    let source = PathBuf::from(
        std::env::var_os("STUDYAPP_TEST_INITIAL_DIR").expect("local dataset directory required"),
    );
    let packages = load_initial(&source).unwrap();
    let mut count = 0;
    for folder in ["android", "basic-cs"] {
        let value: serde_json::Value = serde_json::from_slice(
            &fs::read(source.join(folder).join("grading_examples.json")).unwrap(),
        )
        .unwrap();
        let dataset = value["dataset_id"].as_str().unwrap();
        let package = packages
            .iter()
            .find(|p| p.learning.dataset_id == dataset)
            .unwrap();
        for case in value["cases"].as_array().unwrap() {
            let item = package
                .learning
                .items
                .iter()
                .find(|item| case["item_id"].as_str() == Some(item.id.as_str()))
                .unwrap();
            let actual = exam::grade(
                case["input"].as_str().unwrap(),
                &item.answer,
                &item.accepted_answers,
            )
            .0 == "correct";
            assert_eq!(
                actual,
                case["expected_correct"].as_bool().unwrap(),
                "grading case index {count}"
            );
            count += 1;
        }
    }
    assert_eq!(count, 1705);
}

fn setup() -> (tempfile::TempDir, AppPaths, Store) {
    let temp = tempfile::tempdir().unwrap();
    let paths = super::super::tests::fixture(temp.path());
    let store = Store::open(&paths).unwrap();
    store.connection.execute("INSERT INTO items VALUES ('fixture-a','second',1,'가상 질문 2','B','[]',1),('fixture-a','third',2,'가상 질문 3','C','[]',1)",[]).unwrap();
    store
        .connection
        .execute(
            "INSERT INTO datasets VALUES ('fixture-b','가상 묶음 B',1,1,'fixture-hash',NULL)",
            [],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO items VALUES ('fixture-b','fixture-item',0,'가상 질문 B','D','[]',1)",
            [],
        )
        .unwrap();
    (temp, paths, store)
}
fn request() -> StartExamRequest {
    StartExamRequest {
        exam_id: Uuid::new_v4().to_string(),
        dataset_ids: vec!["fixture-a".into()],
        count_mode: "count".into(),
        count: Some(1),
        order_mode: "ordered".into(),
        review_mode: "general".into(),
    }
}
fn submit(store: &mut Store, id: &str, answers: Vec<String>) {
    store
        .submit_exam(SubmitExamRequest {
            exam_id: id.into(),
            revision: 1,
            current_position: 0,
            responses: answers,
        })
        .unwrap();
}
fn finish_one(store: &mut Store, response: &str) -> ExamResult {
    let exam = store.start_exam(request()).unwrap();
    submit(store, &exam.exam_id, vec![response.into()]);
    store.complete_exam(&exam.exam_id).unwrap();
    store.exam_result(&exam.exam_id).unwrap()
}

#[test]
fn ordered_selection_uses_requested_dataset_order_and_all_mode() {
    let (_temp, _paths, mut store) = setup();
    let mut req = request();
    req.dataset_ids = vec!["fixture-b".into(), "fixture-a".into()];
    req.count_mode = "all".into();
    req.count = None;
    let session = store.start_exam(req).unwrap();
    assert_eq!(session.items.len(), 4);
    assert_eq!(session.items[0].dataset_id, "fixture-b");
    assert_eq!(session.items[1].item_id, "fixture-item");
    assert_eq!(session.items[2].item_id, "second");
    assert_eq!(session.items[3].item_id, "third");
}

#[test]
fn random_selection_is_unique_and_retries_preserve_order() {
    let (_temp, _paths, mut store) = setup();
    let mut req = request();
    req.order_mode = "random".into();
    req.count = Some(2);
    let first = store.start_exam(req.clone()).unwrap();
    let again = store.start_exam(req.clone()).unwrap();
    assert_eq!(first.items.len(), 2);
    assert_ne!(first.items[0].item_id, first.items[1].item_id);
    assert_eq!(
        first.items.iter().map(|i| &i.item_id).collect::<Vec<_>>(),
        again.items.iter().map(|i| &i.item_id).collect::<Vec<_>>()
    );
    req.count = Some(3);
    assert_eq!(
        store.start_exam(req).err().unwrap().code,
        "RevisionConflict"
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM exams", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn invalid_start_inputs_and_empty_candidates_leave_no_exam() {
    let (_temp, _paths, mut store) = setup();
    for kind in 0..9 {
        let mut req = request();
        match kind {
            0 => req.count = Some(0),
            1 => req.count = Some(99),
            2 => req.dataset_ids.push("fixture-a".into()),
            3 => req.dataset_ids = vec!["missing".into()],
            4 => req.exam_id = "BAD-ID".into(),
            5 => req.review_mode = "wrong_only".into(),
            6 => req.count_mode = "all".into(),
            7 => req.dataset_ids.clear(),
            _ => req.order_mode = "other".into(),
        };
        assert!(store.start_exam(req).is_err());
    }
    store
        .connection
        .execute("UPDATE items SET is_active=0", [])
        .unwrap();
    assert_eq!(
        store.start_exam(request()).err().unwrap().code,
        "NoCandidates"
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM exams", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn draft_never_exposes_answers_and_result_requires_completion() {
    let (_temp, _paths, mut store) = setup();
    let started = store.start_exam(request()).unwrap();
    let dto = serde_json::to_value(&started).unwrap();
    for key in [
        "answer",
        "accepted_answers",
        "answer_snapshot",
        "judgement",
        "supplement",
    ] {
        assert!(dto["items"][0].get(key).is_none());
    }
    assert_eq!(
        store.exam_result(&started.exam_id).err().unwrap().code,
        "InvalidState"
    );
    assert_eq!(
        store.complete_exam(&started.exam_id).err().unwrap().code,
        "InvalidState"
    );
    submit(&mut store, &started.exam_id, vec!["alias".into()]);
    assert_eq!(
        store.exam_result(&started.exam_id).err().unwrap().code,
        "InvalidState"
    );
    store.complete_exam(&started.exam_id).unwrap();
    assert_eq!(
        store.exam_result(&started.exam_id).unwrap().correct_count,
        1
    );
}

#[test]
fn grading_uses_saved_snapshot_and_persists_after_reopening() {
    let (_temp, paths, mut store) = setup();
    let started = store.start_exam(request()).unwrap();
    store.connection.execute("UPDATE items SET question='새 가상 질문',answer='changed',accepted_answers_json='[]' WHERE dataset_id='fixture-a' AND item_id='fixture-item'",[]).unwrap();
    submit(&mut store, &started.exam_id, vec!["alias".into()]);
    store.complete_exam(&started.exam_id).unwrap();
    drop(store);
    let mut reopened = Store::open(&paths).unwrap();
    let result = reopened.exam_result(&started.exam_id).unwrap();
    assert_eq!(result.correct_count, 1);
    assert_eq!(result.items[0].answer, "a  b");
    assert_eq!(result.items[0].question, "가상 질문");
    assert_eq!(result.items[0].accepted_answers, vec!["alias"]);
}

#[test]
fn exact_grading_handles_fixed_white_space_without_normalization() {
    for input in ["a  b", "alias"] {
        assert_eq!(
            exam::grade(input, "a  b", &["alias".into()]),
            ("correct", None)
        );
    }
    for input in [
        "a b", "a  b ", "Alias", "ALIAS", "\u{001c}", "\u{001d}", "\u{001e}", "\u{001f}",
        "\u{200b}", "\u{feff}",
    ] {
        assert_eq!(
            exam::grade(input, "a  b", &[]),
            ("incorrect", Some("no_match"))
        );
    }
    for cp in [
        9, 10, 11, 12, 13, 32, 133, 160, 5760, 8192, 8193, 8194, 8195, 8196, 8197, 8198, 8199,
        8200, 8201, 8202, 8232, 8233, 8239, 8287, 12288,
    ] {
        let input = format!(" {} ", char::from_u32(cp).unwrap());
        assert_eq!(
            exam::grade(&input, "a  b", &[]),
            ("incorrect", Some("unanswered"))
        );
    }
    assert_eq!(exam::grade("", "a", &[]), ("incorrect", Some("unanswered")));
    assert_eq!(
        exam::grade("e\u{0301}", "é", &[]),
        ("incorrect", Some("no_match"))
    );
}

#[test]
fn submission_validates_revision_count_position_and_preserves_raw_input() {
    let (_temp, _paths, mut store) = setup();
    let started = store.start_exam(request()).unwrap();
    for (revision, position, responses) in [
        (1, 0, vec![]),
        (1, 1, vec!["x".into()]),
        (0, 0, vec!["changed".into()]),
        (9_007_199_254_740_992, 0, vec!["x".into()]),
    ] {
        assert!(store
            .submit_exam(SubmitExamRequest {
                exam_id: started.exam_id.clone(),
                revision,
                current_position: position,
                responses
            })
            .is_err());
        assert_eq!(
            store.exam_session(&started.exam_id).unwrap().status,
            "in_progress"
        );
    }
    submit(&mut store, &started.exam_id, vec![" \u{0085}\t".into()]);
    // 이미 제출된 동일 시험에 다른 답이 와도 처음 확정한 답변은 잠겨 있다.
    submit(&mut store, &started.exam_id, vec!["alias".into()]);
    store.complete_exam(&started.exam_id).unwrap();
    let result = store.exam_result(&started.exam_id).unwrap();
    assert_eq!(result.items[0].response_text, " \u{0085}\t");
    assert_eq!(result.unanswered_count, 1);
    assert_eq!(result.incorrect_count, 1);
}

#[test]
fn completion_retry_and_result_reads_do_not_duplicate_progress() {
    let (_temp, _paths, mut store) = setup();
    let result = finish_one(&mut store, "wrong");
    for _ in 0..3 {
        store.complete_exam(&result.exam_id).unwrap();
        let again = store.exam_result(&result.exam_id).unwrap();
        assert_eq!(again.completion_order, 1);
    }
    let state: (i64, i64) = store
        .connection
        .query_row(
            "SELECT is_review_target,correct_streak FROM item_progress",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, (1, 0));
    for expected in 1..=3 {
        let next = finish_one(&mut store, "alias");
        assert_eq!(next.items[0].correct_streak, expected);
        assert_eq!(next.items[0].is_review_target, expected < 3);
    }
    let reset = finish_one(&mut store, "wrong");
    assert_eq!(reset.items[0].correct_streak, 0);
    assert!(reset.items[0].is_review_target);
}

#[test]
fn mid_completion_failure_rolls_back_all_judgements_state_and_progress() {
    let (_temp, _paths, mut store) = setup();
    let started = store.start_exam(request()).unwrap();
    submit(&mut store, &started.exam_id, vec!["wrong".into()]);
    store.connection.execute_batch("CREATE TEMP TRIGGER fail_progress BEFORE INSERT ON item_progress BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(store.complete_exam(&started.exam_id).is_err());
    assert_eq!(
        store.exam_session(&started.exam_id).unwrap().status,
        "submitted"
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT judgement FROM exam_responses", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "ungraded"
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM item_progress", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    store
        .connection
        .execute_batch("DROP TRIGGER fail_progress")
        .unwrap();
    store.complete_exam(&started.exam_id).unwrap();
    assert_eq!(
        store
            .exam_result(&started.exam_id)
            .unwrap()
            .completion_order,
        1
    );
}

#[test]
fn failed_submission_is_atomic_and_unknown_rules_preserve_submitted_answers() {
    let (_temp, _paths, mut store) = setup();
    let mut req = request();
    req.count = Some(3);
    let started = store.start_exam(req).unwrap();
    store.connection.execute_batch("CREATE TEMP TRIGGER fail_response BEFORE UPDATE OF response_text ON exam_responses WHEN NEW.position=1 BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(store
        .submit_exam(SubmitExamRequest {
            exam_id: started.exam_id.clone(),
            revision: 1,
            current_position: 0,
            responses: vec!["first".into(), "second".into(), "third".into()]
        })
        .is_err());
    assert!(store
        .exam_session(&started.exam_id)
        .unwrap()
        .items
        .iter()
        .all(|i| i.response_text.is_empty()));
    store
        .connection
        .execute_batch("DROP TRIGGER fail_response")
        .unwrap();
    submit(&mut store, &started.exam_id, vec!["x".into(); 3]);
    store
        .connection
        .execute("UPDATE exams SET grading_rule_version='unknown'", [])
        .unwrap();
    assert_eq!(
        store.complete_exam(&started.exam_id).err().unwrap().code,
        "UnsupportedFormat"
    );
    assert_eq!(
        store.exam_session(&started.exam_id).unwrap().status,
        "submitted"
    );
}

#[test]
fn ownership_and_damaged_position_are_rejected() {
    let (_temp, _paths, mut store) = setup();
    let started = store.start_exam(request()).unwrap();
    let owner = store.learner_id.clone();
    store.learner_id = Uuid::new_v4().to_string();
    assert_eq!(
        store.exam_session(&started.exam_id).err().unwrap().code,
        "OwnershipMismatch"
    );
    store.learner_id = owner;
    store
        .connection
        .execute("UPDATE exam_responses SET position=5", [])
        .unwrap();
    assert_eq!(
        store.exam_session(&started.exam_id).err().unwrap().code,
        "DataInconsistent"
    );
}

#[test]
#[ignore = "실제 자료는 Git 제외. 로컬 자료 환경에서만 실행"]
fn private_local_exam_roundtrip() {
    let source =
        std::env::var_os("STUDYAPP_TEST_INITIAL_DIR").expect("local dataset directory required");
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths {
        db_dir: temp.path().join("db"),
        initial_dir: PathBuf::from(source),
    };
    let mut store = Store::open(&paths).unwrap();
    let datasets = store.list_datasets().unwrap();
    assert_eq!(datasets.len(), 2);
    let req = StartExamRequest {
        dataset_ids: datasets.iter().map(|d| d.dataset_id.clone()).collect(),
        count_mode: "all".into(),
        count: None,
        ..request()
    };
    let started = store.start_exam(req).unwrap();
    assert_eq!(started.items.len(), 254);
    let saved = store
        .save_progress(SubmitExamRequest {
            exam_id: started.exam_id.clone(),
            revision: 1,
            current_position: 253,
            responses: vec![String::new(); 254],
        })
        .unwrap();
    assert_eq!(saved.outcome, "saved");
    assert_eq!(
        store
            .exam_session(&started.exam_id)
            .unwrap()
            .current_position,
        253
    );
    store
        .submit_exam(SubmitExamRequest {
            exam_id: started.exam_id.clone(),
            revision: 2,
            current_position: 253,
            responses: vec![String::new(); 254],
        })
        .unwrap();
    store.complete_exam(&started.exam_id).unwrap();
    let result = store.exam_result(&started.exam_id).unwrap();
    assert_eq!(result.unanswered_count, 254);
    assert_eq!(result.incorrect_count, 254);
    drop(store);
    let mut reopened = Store::open(&paths).unwrap();
    assert_eq!(
        reopened.exam_result(&started.exam_id).unwrap().items.len(),
        254
    );
}

#[test]
fn start_uuid_is_canonical_lowercase_v4() {
    let mut req = request();
    req.exam_id = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA".into();
    assert!(req.validate().is_err());
    req.exam_id = "aaaaaaaa-aaaa-4aaa-caaa-aaaaaaaaaaaa".into();
    assert!(req.validate().is_err());
    req.exam_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into();
    assert!(req.validate().is_ok());
    assert!(serde_json::from_value::<StartExamRequest>(json!({"exam_id":req.exam_id,"dataset_ids":["fixture-a"],"count_mode":"count","count":1.5,"order_mode":"ordered","review_mode":"general"})).is_err());
}
