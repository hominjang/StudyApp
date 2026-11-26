use super::*;

fn setup() -> (tempfile::TempDir, AppPaths, Store, ExamSession) {
    let temp = tempfile::tempdir().unwrap();
    let paths = super::super::tests::fixture(temp.path());
    let mut store = Store::open(&paths).unwrap();
    store.connection.execute("INSERT INTO items VALUES ('fixture-a','second',1,'가상 질문 2','B','[]',1),('fixture-a','third',2,'가상 질문 3','C','[]',1)",[]).unwrap();
    let session = store
        .start_exam(StartExamRequest {
            exam_id: Uuid::new_v4().to_string(),
            dataset_ids: vec!["fixture-a".into()],
            count_mode: "all".into(),
            count: None,
            order_mode: "ordered".into(),
            review_mode: "general".into(),
        })
        .unwrap();
    (temp, paths, store, session)
}
fn snapshot(id: &str, revision: u64) -> SubmitExamRequest {
    SubmitExamRequest {
        exam_id: id.into(),
        revision,
        current_position: 2,
        responses: vec![" a  b ".into(), "\u{0085}".into(), "\u{feff}".into()],
    }
}

#[test]
fn newer_save_wins_and_equal_revision_must_have_identical_content() {
    let (_temp, _paths, mut store, session) = setup();
    let request = snapshot(&session.exam_id, 2);
    assert_eq!(
        store.save_progress(request.clone()).unwrap().outcome,
        "saved"
    );
    assert_eq!(
        store.save_progress(request.clone()).unwrap().outcome,
        "already_saved"
    );
    let mut stale = snapshot(&session.exam_id, 1);
    stale.responses[0] = "old".into();
    assert_eq!(store.save_progress(stale).unwrap().outcome, "stale");
    let mut conflict = request.clone();
    conflict.responses[0] = "other".into();
    assert_eq!(
        store.save_progress(conflict).err().unwrap().code,
        "RevisionConflict"
    );
    let restored = store.exam_session(&session.exam_id).unwrap();
    assert_eq!(restored.revision, 2);
    assert_eq!(restored.current_position, 2);
    assert_eq!(restored.items[0].response_text, request.responses[0]);
}

#[test]
fn saved_progress_reopens_without_source_and_keeps_snapshot_order() {
    let (temp, mut paths, mut store, session) = setup();
    let request = snapshot(&session.exam_id, 1);
    store.save_progress(request.clone()).unwrap();
    store
        .connection
        .execute(
            "UPDATE items SET question='바뀐 가상 질문',answer='changed',is_active=0",
            [],
        )
        .unwrap();
    drop(store);
    paths.initial_dir = temp.path().join("absent");
    let mut reopened = Store::open(&paths).unwrap();
    let restored = reopened.exam_session(&session.exam_id).unwrap();
    assert_eq!(restored.current_position, 2);
    assert_eq!(
        restored
            .items
            .iter()
            .map(|i| &i.question)
            .collect::<Vec<_>>(),
        session
            .items
            .iter()
            .map(|i| &i.question)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        restored
            .items
            .iter()
            .map(|i| &i.response_text)
            .collect::<Vec<_>>(),
        request.responses.iter().collect::<Vec<_>>()
    );
    let pending = reopened.pending_exams().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].answered_count, 2);
    assert_eq!(pending[0].current_position, 2);
    let dto = serde_json::to_value(&pending).unwrap();
    for key in ["items", "question", "answer", "responses", "response_text"] {
        assert!(dto[0].get(key).is_none());
    }
}

#[test]
fn autosave_after_submission_and_completion_is_locked() {
    let (_temp, _paths, mut store, session) = setup();
    let request = snapshot(&session.exam_id, 1);
    store.save_progress(request.clone()).unwrap();
    store.submit_exam(request.clone()).unwrap();
    let mut late = snapshot(&session.exam_id, 9);
    late.responses = vec!["late".into(); 3];
    assert_eq!(store.save_progress(late.clone()).unwrap().outcome, "locked");
    store.complete_exam(&session.exam_id).unwrap();
    assert_eq!(store.save_progress(late).unwrap().outcome, "locked");
    let result = store.exam_result(&session.exam_id).unwrap();
    assert_eq!(result.items[0].response_text, request.responses[0]);
    assert!(store.pending_exams().unwrap().is_empty());
}

#[test]
fn failed_save_rolls_back_responses_position_revision_and_timestamp() {
    let (_temp, _paths, mut store, session) = setup();
    let before: String = store
        .connection
        .query_row("SELECT saved_at FROM exams", [], |r| r.get(0))
        .unwrap();
    store.connection.execute_batch("CREATE TEMP TRIGGER fail_save BEFORE UPDATE OF response_text ON exam_responses WHEN NEW.position=1 BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(store.save_progress(snapshot(&session.exam_id, 1)).is_err());
    let restored = store.exam_session(&session.exam_id).unwrap();
    assert_eq!(restored.revision, 0);
    assert_eq!(restored.current_position, 0);
    assert!(restored.items.iter().all(|i| i.response_text.is_empty()));
    let after: String = store
        .connection
        .query_row("SELECT saved_at FROM exams", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, after);
    store
        .connection
        .execute_batch("DROP TRIGGER fail_save")
        .unwrap();
    assert_eq!(
        store
            .save_progress(snapshot(&session.exam_id, 1))
            .unwrap()
            .outcome,
        "saved"
    );
}

#[test]
fn pending_submitted_exam_can_finish_after_restart_exactly_once() {
    let (_temp, paths, mut store, session) = setup();
    store.submit_exam(snapshot(&session.exam_id, 1)).unwrap();
    drop(store);
    let mut reopened = Store::open(&paths).unwrap();
    let pending = reopened.pending_exams().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].status, "submitted");
    assert_eq!(
        reopened.exam_result(&session.exam_id).err().unwrap().code,
        "InvalidState"
    );
    reopened.complete_exam(&session.exam_id).unwrap();
    reopened.complete_exam(&session.exam_id).unwrap();
    assert_eq!(
        reopened
            .exam_result(&session.exam_id)
            .unwrap()
            .completion_order,
        1
    );
    assert!(reopened.pending_exams().unwrap().is_empty());
}

#[test]
fn save_checks_owner_shape_and_safe_integer_revision() {
    let (_temp, _paths, mut store, session) = setup();
    for kind in 0..3 {
        let mut request = snapshot(&session.exam_id, 1);
        match kind {
            0 => request.current_position = 3,
            1 => request.responses.clear(),
            _ => request.revision = 9_007_199_254_740_992,
        };
        assert_eq!(
            store.save_progress(request).err().unwrap().code,
            "ValidationError"
        );
    }
    store.learner_id = Uuid::new_v4().to_string();
    assert_eq!(
        store
            .save_progress(snapshot(&session.exam_id, 1))
            .err()
            .unwrap()
            .code,
        "OwnershipMismatch"
    );
    assert!(store.pending_exams().unwrap().is_empty());
}
