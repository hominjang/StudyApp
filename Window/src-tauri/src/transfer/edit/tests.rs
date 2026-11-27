use super::*;
use crate::transfer::tests::fixture;
use crate::{
    exam::{StartExamRequest, SubmitExamRequest},
    runtime_store::RuntimeStore,
    study_material::StudyRequest,
    worker::DbWorker,
};
use serde_json::json;

fn request() -> EditMaterialRequest {
    EditMaterialRequest {
        request_id: Uuid::new_v4().to_string(),
        dataset_id: "seed".into(),
        item_id: "a".into(),
        expected_revision: 1,
        criteria: Criteria::ShortAnswer {
            answer: "new  answer".into(),
            accepted_answers: vec!["alias".into()],
        },
        explanation: Some("Synthetic new explanation\r\nsecond line".into()),
    }
}
fn save(
    s: &mut RuntimeStore,
    state: &mut MaterialEditState,
    r: EditMaterialRequest,
) -> AppResult<EditMaterialResult> {
    let (db, v) = s.transfer_connection();
    state.save(db, v, r)
}
fn read(s: &mut RuntimeStore) -> crate::study_material::StudyPage {
    s.study_materials(StudyRequest {
        dataset_id: Some("seed".into()),
        search: String::new(),
        offset: 0,
        limit: 20,
    })
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
fn both_versions_edit_answers_and_explanation_reopen_and_preserve_attempts_and_progress() {
    for version in [1, 2] {
        let (_root, paths, mut s) = fixture(version);
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
        let snapshot = serde_json::to_value(s.exam_session(&pending).unwrap()).unwrap();
        let progress_before = s.list_datasets().unwrap()[0].review_target_count;
        let mut edits = MaterialEditState::default();
        let result = save(&mut s, &mut edits, request()).unwrap();
        assert_eq!(result.dataset_revision, 2);
        let data = read(&mut s);
        assert_eq!(data.items[0].dataset_revision, 2);
        assert_eq!(
            data.items[0].explanation.as_deref(),
            Some("Synthetic new explanation\r\nsecond line")
        );
        assert!(
            matches!(&data.items[0].criteria,Criteria::ShortAnswer{answer,..} if answer=="new  answer")
        );
        assert_eq!(
            serde_json::to_value(s.exam_session(&pending).unwrap()).unwrap(),
            snapshot
        );
        assert_eq!(s.exam_session(&completed).unwrap().status, "completed");
        assert_eq!(
            s.list_datasets().unwrap()[0].review_target_count,
            progress_before
        );
        let future = begin(&mut s);
        let answer:String=s.transfer_connection().0.query_row(if version==1 {"SELECT answer_snapshot FROM exam_responses WHERE exam_id=?1 AND position=0"} else {"SELECT json_extract(criteria_snapshot_json,'$.answer') FROM exam_responses WHERE exam_id=?1 AND position=0"},[future],|r|r.get(0)).unwrap();
        assert_eq!(answer, "new  answer");
        drop(s);
        assert_eq!(
            read(&mut RuntimeStore::open(&paths).unwrap()).items[0].dataset_revision,
            2
        );
    }
}
#[test]
fn successful_retry_is_idempotent_and_reusing_request_id_with_other_payload_is_rejected() {
    let (_root, _paths, mut s) = fixture(2);
    let mut edits = MaterialEditState::default();
    let r = request();
    save(&mut s, &mut edits, r.clone()).unwrap();
    let changes = s.transfer_connection().0.total_changes();
    save(&mut s, &mut edits, r.clone()).unwrap();
    assert_eq!(s.transfer_connection().0.total_changes(), changes);
    let mut different = r;
    different.explanation = None;
    assert_eq!(
        save(&mut s, &mut edits, different).err().unwrap().code,
        "RequestConflict"
    );
    assert_eq!(read(&mut s).items[0].dataset_revision, 2);
}
#[test]
fn stale_revision_invalid_answer_type_change_and_inactive_item_do_not_write() {
    let (_root, _paths, mut s) = fixture(2);
    let mut edits = MaterialEditState::default();
    save(&mut s, &mut edits, request()).unwrap();
    let changes = s.transfer_connection().0.total_changes();
    assert_eq!(
        save(&mut s, &mut edits, request()).err().unwrap().code,
        "MaterialConflict"
    );
    for criteria in [
        Criteria::ShortAnswer {
            answer: "one two three four five".into(),
            accepted_answers: vec![],
        },
        Criteria::ShortAnswer {
            answer: "valid".into(),
            accepted_answers: vec!["\u{0085}".into()],
        },
        Criteria::Descriptive {
            reference_answer: "Synthetic reference".into(),
            key_points: vec!["point".into()],
        },
    ] {
        let mut r = request();
        r.expected_revision = 2;
        r.criteria = criteria;
        assert!(save(&mut s, &mut edits, r).is_err());
    }
    assert_eq!(s.transfer_connection().0.total_changes(), changes);
    s.transfer_connection()
        .0
        .execute("UPDATE items SET is_active=0 WHERE item_id='a'", [])
        .unwrap();
    let mut r = request();
    r.expected_revision = 2;
    assert!(save(&mut s, &mut edits, r).is_err());
}
#[test]
fn transaction_failure_rolls_back_and_same_request_can_retry() {
    for version in [1, 2] {
        let (_root, _paths, mut s) = fixture(version);
        let mut edits = MaterialEditState::default();
        let r = request();
        s.transfer_connection().0.execute_batch("CREATE TRIGGER synthetic_edit_fail BEFORE UPDATE ON datasets BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
        assert!(save(&mut s, &mut edits, r.clone()).is_err());
        let page = read(&mut s);
        assert_eq!(page.items[0].dataset_revision, 1);
        assert!(page.items[0].explanation.is_none());
        assert!(
            matches!(&page.items[0].criteria,Criteria::ShortAnswer{answer,..} if answer=="alpha")
        );
        s.transfer_connection()
            .0
            .execute_batch("DROP TRIGGER synthetic_edit_fail")
            .unwrap();
        save(&mut s, &mut edits, r).unwrap();
        assert_eq!(read(&mut s).items[0].dataset_revision, 2);
    }
}
#[test]
fn supplements_and_links_are_preserved_and_edit_invalidates_import_preview() {
    let (_root, _paths, mut s) = fixture(2);
    let (db, _) = s.transfer_connection();
    db.execute("INSERT INTO supplements VALUES ('seed','a',?1)",[json!({"explanation":"old","source_refs":[{"dataset_id":"Original_ID","item_id":"Original Item"}],"tags":["synthetic"]}).to_string()]).unwrap();
    db.execute(
        "INSERT INTO item_learning_links VALUES ('seed','a',0,?1)",
        [
            json!({"item_id":"a","prerequisites":[{"dataset_id":"seed","item_id":"b"}]})
                .to_string(),
        ],
    )
    .unwrap();
    let mut imports = ImportState::default();
    let p=imports.preview(db,2,ImportRequest{learning_json:json!({"schema_version":"1.0","dataset_id":"seed","title":"Synthetic updated","items":[{"id":"a","question":"Q","answer":"x"}]}).to_string(),supplement_json:None}).unwrap();
    let mut edits = MaterialEditState::default();
    let mut r = request();
    r.explanation = None;
    save(&mut s, &mut edits, r).unwrap();
    let (db, _) = s.transfer_connection();
    let payload: String = db
        .query_row(
            "SELECT payload_json FROM supplements WHERE item_id='a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let payload: Value = serde_json::from_str(&payload).unwrap();
    assert!(payload.get("explanation").is_none());
    assert!(payload.get("source_refs").is_some());
    assert!(payload.get("tags").is_some());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM item_learning_links", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        imports.apply(db, 2, &p.token).err().unwrap().code,
        "ImportConflict"
    );
}
#[test]
fn unchanged_data_is_noop_and_descriptive_editor_preserves_reference_and_key_points() {
    let (_root, _paths, mut s) = fixture(2);
    let mut edits = MaterialEditState::default();
    let mut r = request();
    r.criteria = read(&mut s).items[0].criteria.clone();
    r.explanation = None;
    assert!(!save(&mut s, &mut edits, r).unwrap().changed);
    assert_eq!(read(&mut s).items[0].dataset_revision, 1);
    let (db, _) = s.transfer_connection();
    db.execute("UPDATE items SET type='descriptive',criteria_json=?1 WHERE item_id='a'",[json!({"type":"descriptive","reference_answer":"old reference","key_points":["old point"]}).to_string()]).unwrap();
    let mut r = request();
    r.criteria = Criteria::Descriptive {
        reference_answer: "Synthetic reference with more than four words\r\nnew line".into(),
        key_points: vec!["updated point".into()],
    };
    save(&mut s, &mut edits, r).unwrap();
    assert!(
        matches!(&read(&mut s).items[0].criteria,Criteria::Descriptive{reference_answer,key_points} if reference_answer.contains("\r\n") && key_points==&vec!["updated point"])
    );
}
#[test]
fn worker_connects_material_edit_and_read_revision() {
    let (_root, paths, s) = fixture(1);
    drop(s);
    let worker = DbWorker::start(paths).unwrap();
    worker.edit_material(request()).unwrap();
    assert_eq!(
        worker
            .study_materials(StudyRequest {
                dataset_id: Some("seed".into()),
                search: String::new(),
                offset: 0,
                limit: 20
            })
            .unwrap()
            .items[0]
            .dataset_revision,
        2
    );
}
