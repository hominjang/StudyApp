use super::*;
use crate::{
    exam::{StartExamRequest, SubmitExamRequest},
    runtime_store::RuntimeStore,
    storage::{AppPaths, Store},
    transfer::{ImportRequest, ImportState},
    worker::DbWorker,
};
use serde_json::json;
use std::fs;
use uuid::Uuid;

fn fixture(version: u8) -> (tempfile::TempDir, AppPaths, RuntimeStore) {
    let root = tempfile::tempdir().unwrap();
    let initial = root.path().join("initial");
    fs::create_dir(&initial).unwrap();
    let items:Vec<_>=(0..22).map(|i|json!({"id":format!("i-{i}"),"question":if i==0 {"Synthetic %_ ' question".into()} else {format!("Synthetic question {i}")},"answer":"alpha  beta","accepted_answers":["A","a b"]})).collect();
    fs::write(
        initial.join("one.json"),
        json!({"schema_version":"1.0","dataset_id":"one","title":"Synthetic one","items":items})
            .to_string(),
    )
    .unwrap();
    fs::write(initial.join("two.json"),json!({"schema_version":"1.0","dataset_id":"two","title":"Synthetic two","items":[{"id":"i-0","question":"Other question","answer":"other"}]}).to_string()).unwrap();
    fs::write(initial.join("extra.json"),json!({"schema_version":"1.0","dataset_id":"one","items":[{"id":"i-0","explanation":"Synthetic explanation\nsecond line"}]}).to_string()).unwrap();
    fs::write(initial.join("catalog.json"),json!({"schema_version":"1.0","datasets":[{"dataset_id":"one","title":"Synthetic one","learning_file":"one.json","supplement_file":"extra.json","question_count":22},{"dataset_id":"two","title":"Synthetic two","learning_file":"two.json","question_count":1}]}).to_string()).unwrap();
    let paths = AppPaths {
        initial_dir: initial,
        db_dir: root.path().join("db"),
    };
    if version == 1 {
        drop(Store::open(&paths).unwrap());
    }
    let store = RuntimeStore::open(&paths).unwrap();
    (root, paths, store)
}
fn request() -> StudyRequest {
    StudyRequest {
        dataset_id: None,
        search: String::new(),
        offset: 0,
        limit: 20,
    }
}
fn start(s: &mut RuntimeStore) -> String {
    s.start_exam(StartExamRequest {
        exam_id: Uuid::new_v4().to_string(),
        dataset_ids: vec!["two".into()],
        count_mode: "all".into(),
        count: None,
        order_mode: "ordered".into(),
        review_mode: "general".into(),
    })
    .unwrap()
    .exam_id
}

#[test]
fn both_store_versions_reveal_current_answers_without_modifying_completed_or_pending_exams() {
    for version in [1, 2] {
        let (_root, _paths, mut s) = fixture(version);
        let completed = start(&mut s);
        s.submit_exam(SubmitExamRequest {
            exam_id: completed.clone(),
            revision: 1,
            current_position: 0,
            responses: vec!["wrong".into()],
        })
        .unwrap();
        s.complete_exam(&completed).unwrap();
        let pending = start(&mut s);
        let before = serde_json::to_value(s.exam_session(&pending).unwrap()).unwrap();
        assert!(before["items"][0].get("answer").is_none());
        assert!(before["items"][0].get("criteria").is_none());
        let changes = s.transfer_connection().0.total_changes();
        let p = s.study_materials(request()).unwrap();
        assert_eq!(p.total, 23);
        assert_eq!(p.items.len(), 20);
        assert!(
            matches!(&p.items[0].criteria,Criteria::ShortAnswer{answer,accepted_answers} if answer=="alpha  beta" && accepted_answers==&vec!["A","a b"])
        );
        assert_eq!(
            p.items[0].explanation.as_deref(),
            Some("Synthetic explanation\nsecond line")
        );
        assert_eq!(s.transfer_connection().0.total_changes(), changes);
        assert_eq!(
            serde_json::to_value(s.exam_session(&pending).unwrap()).unwrap(),
            before
        );
        assert_eq!(s.exam_session(&completed).unwrap().status, "completed");
        assert_eq!(s.list_datasets().unwrap()[1].review_target_count, 1);
    }
}
#[test]
fn stable_paging_and_scope_keep_identical_item_ids_in_different_datasets_distinct() {
    let (_root, _paths, mut s) = fixture(2);
    let first = s.study_materials(request()).unwrap();
    let last = s
        .study_materials(StudyRequest {
            offset: 20,
            ..request()
        })
        .unwrap();
    assert_eq!((last.total, last.items.len()), (23, 3));
    assert_eq!(last.items[2].dataset_id, "two");
    assert_eq!(first.items[0].item_id, last.items[2].item_id);
    let scoped = s
        .study_materials(StudyRequest {
            dataset_id: Some("two".into()),
            ..request()
        })
        .unwrap();
    assert_eq!(scoped.total, 1);
    assert_eq!(scoped.items[0].dataset_id, "two");
}
#[test]
fn question_search_is_literal_and_does_not_search_answers_or_user_records() {
    let (_root, _paths, mut s) = fixture(1);
    for term in ["%_", "'", "SYNTHETIC %_"] {
        let p = s
            .study_materials(StudyRequest {
                search: term.into(),
                ..request()
            })
            .unwrap();
        assert_eq!(p.total, 1);
    }
    for term in ["alpha", "' OR 1=1 --"] {
        let p = s
            .study_materials(StudyRequest {
                search: term.into(),
                ..request()
            })
            .unwrap();
        assert_eq!(p.total, 0);
        assert!(p.items.is_empty());
    }
}
#[test]
fn inactive_items_are_hidden_and_out_of_range_page_is_clamped_including_zero_results() {
    for version in [1, 2] {
        let (_root, _paths, mut s) = fixture(version);
        s.transfer_connection()
            .0
            .execute("UPDATE items SET is_active=0 WHERE dataset_id='two'", [])
            .unwrap();
        let p = s
            .study_materials(StudyRequest {
                offset: 200,
                ..request()
            })
            .unwrap();
        assert_eq!((p.total, p.offset, p.items.len()), (22, 20, 2));
        let p = s
            .study_materials(StudyRequest {
                dataset_id: Some("two".into()),
                offset: 200,
                ..request()
            })
            .unwrap();
        assert_eq!((p.total, p.offset), (0, 0));
        assert!(p.items.is_empty());
    }
}
#[test]
fn malformed_scope_page_size_and_search_are_rejected_without_writes() {
    let (_root, _paths, mut s) = fixture(2);
    let changes = s.transfer_connection().0.total_changes();
    for r in [
        StudyRequest {
            limit: 0,
            ..request()
        },
        StudyRequest {
            limit: 51,
            ..request()
        },
        StudyRequest {
            offset: 1,
            ..request()
        },
        StudyRequest {
            dataset_id: Some("Bad_ID".into()),
            ..request()
        },
        StudyRequest {
            search: "x".repeat(513),
            ..request()
        },
        StudyRequest {
            dataset_id: Some("missing".into()),
            ..request()
        },
    ] {
        assert!(s.study_materials(r).is_err());
    }
    assert_eq!(s.transfer_connection().0.total_changes(), changes);
}
#[test]
fn descriptive_reference_and_key_points_preserved_and_corruption_is_not_returned_as_empty_answer() {
    let (_root, _paths, mut s) = fixture(2);
    let (db, v) = s.transfer_connection();
    let mut imports = ImportState::default();
    let data=json!({"schema_version":"2.0","dataset_id":"mixed","title":"Synthetic mixed","items":[{"type":"descriptive","id":"i-0","question":"Synthetic description","reference_answer":"Synthetic reference with more than four words\r\nsecond line","key_points":["Point one","Point two"]}]}).to_string();
    let p = imports
        .preview(
            db,
            v,
            ImportRequest {
                learning_json: data,
                supplement_json: None,
            },
        )
        .unwrap();
    imports.apply(db, v, &p.token).unwrap();
    let page = s
        .study_materials(StudyRequest {
            dataset_id: Some("mixed".into()),
            ..request()
        })
        .unwrap();
    assert!(
        matches!(&page.items[0].criteria,Criteria::Descriptive{reference_answer,key_points} if reference_answer.contains("\r\n") && key_points.len()==2)
    );
    s.transfer_connection()
        .0
        .execute(
            "UPDATE items SET criteria_json='{\"type\":\"descriptive\"}' WHERE dataset_id='mixed'",
            [],
        )
        .unwrap();
    let changes = s.transfer_connection().0.total_changes();
    assert!(s
        .study_materials(StudyRequest {
            dataset_id: Some("mixed".into()),
            ..request()
        })
        .is_err());
    assert_eq!(s.transfer_connection().0.total_changes(), changes);
}
#[test]
fn worker_connects_reading_command_without_creating_attempts() {
    let (_root, paths, s) = fixture(2);
    drop(s);
    let worker = DbWorker::start(paths).unwrap();
    assert_eq!(worker.study_materials(request()).unwrap().total, 23);
    assert!(worker.pending_exams().unwrap().is_empty());
    assert!(worker.exam_history("all".into()).unwrap().is_empty());
}
