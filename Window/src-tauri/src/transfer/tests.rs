use super::*;
use crate::{
    exam::{StartExamRequest, SubmitExamRequest},
    runtime_store::RuntimeStore,
    storage::{AppPaths, Store},
    worker::DbWorker,
};
use serde_json::json;
use std::fs;

// All records below are synthetic and created in disposable directories.
fn source(id: &str) -> Value {
    json!({"schema_version":"1.0","dataset_id":id,"title":"Synthetic", "items":[{"id":"a","question":"Synthetic A","answer":"alpha"},{"id":"b","question":"Synthetic B","answer":"beta"}]})
}
pub(super) fn fixture(version: u8) -> (tempfile::TempDir, AppPaths, RuntimeStore) {
    let root = tempfile::tempdir().unwrap();
    let initial = root.path().join("initial");
    fs::create_dir(&initial).unwrap();
    fs::write(initial.join("learning.json"), source("seed").to_string()).unwrap();
    fs::write(initial.join("catalog.json"),json!({"schema_version":"1.0","datasets":[{"dataset_id":"seed","title":"Synthetic","learning_file":"learning.json","question_count":2}]}).to_string()).unwrap();
    let paths = AppPaths {
        db_dir: root.path().join("db"),
        initial_dir: initial,
    };
    if version == 1 {
        drop(Store::open(&paths).unwrap());
    }
    let store = RuntimeStore::open(&paths).unwrap();
    (root, paths, store)
}
fn preview(
    state: &mut ImportState,
    s: &mut RuntimeStore,
    input: Value,
    extra: Option<Value>,
) -> AppResult<ImportPreview> {
    let (db, v) = s.transfer_connection();
    state.preview(
        db,
        v,
        ImportRequest {
            learning_json: input.to_string(),
            supplement_json: extra.map(|v| v.to_string()),
        },
    )
}
fn apply(
    state: &mut ImportState,
    s: &mut RuntimeStore,
    p: &ImportPreview,
) -> AppResult<ImportResult> {
    let (db, v) = s.transfer_connection();
    state.apply(db, v, &p.token)
}
fn number(s: &mut RuntimeStore, sql: &str) -> i64 {
    s.transfer_connection()
        .0
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}
fn begin(s: &mut RuntimeStore) -> String {
    s.start_exam(StartExamRequest {
        exam_id: Uuid::new_v4().to_string(),
        dataset_ids: vec!["seed".into()],
        count_mode: "all".into(),
        count: None,
        order_mode: "ordered".into(),
        review_mode: "general".into(),
    })
    .unwrap()
    .exam_id
}

#[test]
fn both_versions_preview_without_writing_apply_once_and_reopen() {
    for version in [1, 2] {
        let (_root, paths, mut s) = fixture(version);
        let mut imports = ImportState::default();
        let p = preview(&mut imports, &mut s, source("added"), None).unwrap();
        assert!(!p.existing);
        assert_eq!(p.added, 2);
        assert_eq!(s.status().unwrap().dataset_count, 1);
        apply(&mut imports, &mut s, &p).unwrap();
        apply(&mut imports, &mut s, &p).unwrap();
        assert_eq!(s.status().unwrap().question_count, 4);
        assert_eq!(
            number(
                &mut s,
                "SELECT revision FROM datasets WHERE dataset_id='added'"
            ),
            1
        );
        drop(s);
        fs::remove_dir_all(&paths.initial_dir).unwrap();
        assert_eq!(
            RuntimeStore::open(&paths)
                .unwrap()
                .status()
                .unwrap()
                .question_count,
            4
        );
    }
}
#[test]
fn identical_import_is_noop_even_after_restarting_import_service() {
    for version in [1, 2] {
        let (_root, _paths, mut s) = fixture(version);
        let mut imports = ImportState::default();
        let p = preview(&mut imports, &mut s, source("seed"), None).unwrap();
        assert!(!p.changed);
        assert!(!apply(&mut imports, &mut s, &p).unwrap().changed);
        assert_eq!(
            number(
                &mut s,
                "SELECT revision FROM datasets WHERE dataset_id='seed'"
            ),
            1
        );
    }
}
#[test]
fn updates_preserve_completed_and_pending_snapshots_and_learning_records() {
    for version in [1, 2] {
        let (_root, _paths, mut s) = fixture(version);
        let completed = begin(&mut s);
        s.submit_exam(SubmitExamRequest {
            exam_id: completed.clone(),
            revision: 1,
            current_position: 0,
            responses: vec!["bad".into(), "beta".into()],
        })
        .unwrap();
        s.complete_exam(&completed).unwrap();
        let pending = begin(&mut s);
        let before = serde_json::to_value(s.exam_session(&pending).unwrap()).unwrap();
        let mut changed = source("seed");
        changed["title"] = json!("Updated synthetic");
        changed["items"] = json!([{"id":"b","question":"Changed synthetic","answer":"changed"},{"id":"c","question":"New synthetic","answer":"new"}]);
        let mut imports = ImportState::default();
        let p = preview(&mut imports, &mut s, changed, None).unwrap();
        assert_eq!((p.added, p.modified, p.deactivated), (1, 1, 1));
        apply(&mut imports, &mut s, &p).unwrap();
        assert_eq!(
            serde_json::to_value(s.exam_session(&pending).unwrap()).unwrap(),
            before
        );
        assert_eq!(s.exam_session(&completed).unwrap().status, "completed");
        assert_eq!(number(&mut s, "SELECT COUNT(*) FROM exams"), 2);
        assert_eq!(number(&mut s, "SELECT COUNT(*) FROM item_progress"), 2);
        assert_eq!(
            number(&mut s, "SELECT COUNT(*) FROM items WHERE is_active=0"),
            1
        );
        assert_eq!(
            number(
                &mut s,
                "SELECT COUNT(*) FROM exam_responses WHERE question_snapshot='Synthetic A'"
            ),
            2
        );
    }
}
#[test]
fn reorder_works_without_unique_position_collision() {
    let (_root, _paths, mut s) = fixture(2);
    let mut imports = ImportState::default();
    let mut input = source("seed");
    input["items"].as_array_mut().unwrap().reverse();
    let p = preview(&mut imports, &mut s, input, None).unwrap();
    assert!(p.changed);
    assert_eq!(p.modified, 0);
    apply(&mut imports, &mut s, &p).unwrap();
    assert_eq!(
        number(&mut s, "SELECT position FROM items WHERE item_id='b'"),
        0
    );
}
#[test]
fn failure_rolls_back_and_same_preview_can_retry() {
    for version in [1, 2] {
        let (_root, _paths, mut s) = fixture(version);
        let mut imports = ImportState::default();
        let p = preview(&mut imports, &mut s, source("added"), None).unwrap();
        s.transfer_connection().0.execute_batch("CREATE TRIGGER synthetic_fail BEFORE INSERT ON items BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(apply(&mut imports, &mut s, &p).is_err());
        assert_eq!(s.status().unwrap().dataset_count, 1);
        s.transfer_connection()
            .0
            .execute_batch("DROP TRIGGER synthetic_fail")
            .unwrap();
        apply(&mut imports, &mut s, &p).unwrap();
        assert_eq!(s.status().unwrap().dataset_count, 2);
    }
}
#[test]
fn stale_or_unknown_preview_rejected_without_changes() {
    let (_root, _paths, mut s) = fixture(2);
    let mut imports = ImportState::default();
    let mut input = source("seed");
    input["title"] = json!("Changed");
    let p = preview(&mut imports, &mut s, input, None).unwrap();
    s.transfer_connection()
        .0
        .execute(
            "UPDATE datasets SET revision=revision+1 WHERE dataset_id='seed'",
            [],
        )
        .unwrap();
    assert_eq!(
        apply(&mut imports, &mut s, &p).err().unwrap().code,
        "ImportConflict"
    );
    assert_eq!(
        number(
            &mut s,
            "SELECT COUNT(*) FROM datasets WHERE title='Changed'"
        ),
        0
    );
    let (db, v) = s.transfer_connection();
    assert!(imports.apply(db, v, "unknown").is_err());
}
#[test]
fn invalid_replacement_discards_previous_preview_and_never_mutates() {
    let (_root, _paths, mut s) = fixture(2);
    let mut imports = ImportState::default();
    let p = preview(&mut imports, &mut s, source("added"), None).unwrap();
    let mut invalid = source("bad_ID");
    assert!(preview(&mut imports, &mut s, invalid.clone(), None).is_err());
    assert!(apply(&mut imports, &mut s, &p).is_err());
    invalid = source("added");
    invalid["items"][0]["answer"] = json!("one two three four five");
    assert!(preview(&mut imports, &mut s, invalid, None).is_err());
    let wrong = json!({"schema_version":"1.0","dataset_id":"other","items":[]});
    assert!(preview(&mut imports, &mut s, source("added"), Some(wrong)).is_err());
    assert_eq!(s.status().unwrap().dataset_count, 1);
}
#[test]
fn supplements_metadata_and_links_preserved_when_omitted_and_replaced_when_provided() {
    let (_root, _paths, mut s) = fixture(2);
    let mut imports = ImportState::default();
    let mut input = source("added");
    input["schema_version"] = json!("2.0");
    for item in input["items"].as_array_mut().unwrap() {
        item["type"] = json!("short_answer");
    }
    let extra = json!({"schema_version":"2.0","dataset_id":"added","dataset_metadata":{"note":"Synthetic"},"items":[{"id":"a","explanation":"Synthetic detail"}],"learning_links":[{"item_id":"b","prerequisites":[{"dataset_id":"added","item_id":"a"}]}]});
    let p = preview(&mut imports, &mut s, input.clone(), Some(extra)).unwrap();
    apply(&mut imports, &mut s, &p).unwrap();
    let p = preview(&mut imports, &mut s, input.clone(), None).unwrap();
    assert!(!p.changed);
    apply(&mut imports, &mut s, &p).unwrap();
    let extra = json!({"schema_version":"2.0","dataset_id":"added","items":[{"id":"a","note":"Replacement synthetic"}],"learning_links":[]});
    let p = preview(&mut imports, &mut s, input, Some(extra)).unwrap();
    apply(&mut imports, &mut s, &p).unwrap();
    assert_eq!(
        number(&mut s, "SELECT COUNT(*) FROM item_learning_links"),
        0
    );
    assert_eq!(number(&mut s,"SELECT COUNT(*) FROM supplements WHERE json_extract(payload_json,'$.explanation') IS NOT NULL"),0);
    assert_eq!(
        number(
            &mut s,
            "SELECT COUNT(*) FROM datasets WHERE metadata_json IS NOT NULL"
        ),
        1
    );
}
#[test]
fn mixed_package_supported_in_v2_but_legacy_store_rejects_without_upgrade() {
    for version in [1, 2] {
        let (_root, paths, mut s) = fixture(version);
        let mut imports = ImportState::default();
        let mixed = json!({"schema_version":"2.0","dataset_id":"mixed","title":"Synthetic mixed","items":[{"type":"descriptive","id":"d","question":"Synthetic description","reference_answer":"more than four synthetic reference words","key_points":["Synthetic point"]}]});
        let p = preview(&mut imports, &mut s, mixed, None);
        if version == 1 {
            assert_eq!(p.err().unwrap().code, "UnsupportedFormat");
            assert_eq!(s.status().unwrap().dataset_count, 1);
        } else {
            let p = p.unwrap();
            apply(&mut imports, &mut s, &p).unwrap();
            assert_eq!(s.status().unwrap().question_count, 3);
        }
        drop(s);
        assert_eq!(
            RuntimeStore::open(&paths).unwrap().transfer_connection().1,
            version
        );
    }
}
#[test]
fn worker_connects_preview_apply_and_refreshed_list() {
    let (_root, paths, s) = fixture(2);
    drop(s);
    let worker = DbWorker::start(paths).unwrap();
    let p = worker
        .preview_import(ImportRequest {
            learning_json: source("added").to_string(),
            supplement_json: None,
        })
        .unwrap();
    worker.apply_import(p.token.clone()).unwrap();
    worker.apply_import(p.token).unwrap();
    assert_eq!(worker.datasets().unwrap().len(), 2);
}
