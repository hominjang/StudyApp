use super::*;
use crate::exam::{ExamResult, StartExamRequest, SubmitExamRequest};

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
fn request(mode: &str) -> StartExamRequest {
    StartExamRequest {
        exam_id: Uuid::new_v4().to_string(),
        dataset_ids: vec!["fixture-a".into()],
        count_mode: "all".into(),
        count: None,
        order_mode: "ordered".into(),
        review_mode: mode.into(),
    }
}
fn finish(store: &mut Store, req: StartExamRequest, answers: &[&str]) -> ExamResult {
    let started = store.start_exam(req).unwrap();
    store
        .submit_exam(SubmitExamRequest {
            exam_id: started.exam_id.clone(),
            revision: 1,
            current_position: 0,
            responses: answers.iter().map(|s| s.to_string()).collect(),
        })
        .unwrap();
    store.complete_exam(&started.exam_id).unwrap();
    store.exam_result(&started.exam_id).unwrap()
}
fn mark_third(store: &mut Store) -> ExamResult {
    finish(store, request("general"), &["alias", "B", "wrong"])
}
fn preview(store: &mut Store, ids: &[&str], mode: &str) -> CandidatePreview {
    store
        .preview_candidates(CandidateRequest {
            dataset_ids: ids.iter().map(|s| s.to_string()).collect(),
            review_mode: mode.into(),
        })
        .unwrap()
}

#[test]
fn selection_prioritizes_membership_then_applies_display_order() {
    let flags = [false, false, true, false, true];
    let choose = |mode, order, count| {
        exam::select_indices(&flags, mode, order, count, &mut |indices| indices.reverse()).unwrap()
    };
    assert_eq!(choose("wrong_first", "ordered", Some(3)), vec![0, 2, 4]);
    assert_eq!(choose("wrong_first", "ordered", Some(1)), vec![2]);
    assert_eq!(choose("wrong_only", "ordered", None), vec![2, 4]);
    assert_eq!(choose("wrong_first", "random", Some(3)), vec![3, 2, 4]);
    assert_eq!(choose("general", "random", Some(2)), vec![4, 3]);
    assert_eq!(choose("wrong_only", "random", Some(1)), vec![4]);
    assert_eq!(choose("wrong_first", "ordered", None), vec![0, 1, 2, 3, 4]);
}

#[test]
fn wrong_only_and_wrong_first_use_current_completed_progress() {
    let (_temp, _paths, mut store) = setup();
    mark_third(&mut store);
    assert_eq!(
        preview(&mut store, &["fixture-a"], "general").candidate_count,
        3
    );
    assert_eq!(
        preview(&mut store, &["fixture-a"], "wrong_only").candidate_count,
        1
    );
    let mut req = request("wrong_first");
    req.count_mode = "count".into();
    req.count = Some(2);
    let mixed = store.start_exam(req).unwrap();
    assert_eq!(
        mixed
            .items
            .iter()
            .map(|i| i.item_id.as_str())
            .collect::<Vec<_>>(),
        vec!["fixture-item", "third"]
    );
    let only = store.start_exam(request("wrong_only")).unwrap();
    assert_eq!(only.items.len(), 1);
    assert_eq!(only.items[0].item_id, "third");
    // 진행 중에 빈 답이 있어도 완료 전에는 새로운 오답 대상으로 집계하지 않는다.
    assert_eq!(
        preview(&mut store, &["fixture-a"], "wrong_only").review_target_count,
        1
    );
}

#[test]
fn scope_is_dataset_and_item_pair_and_inactive_items_are_excluded() {
    let (_temp, _paths, mut store) = setup();
    let mut other = request("general");
    other.dataset_ids = vec!["fixture-b".into()];
    finish(&mut store, other, &[""]);
    mark_third(&mut store);
    assert_eq!(
        preview(&mut store, &["fixture-a"], "wrong_only").candidate_count,
        1
    );
    assert_eq!(
        preview(&mut store, &["fixture-b"], "wrong_only").candidate_count,
        1
    );
    let a = store.start_exam(request("wrong_only")).unwrap();
    assert!(a.items.iter().all(|i| i.dataset_id == "fixture-a"));
    assert_eq!(a.items[0].item_id, "third");
    store
        .connection
        .execute(
            "UPDATE items SET is_active=0 WHERE dataset_id='fixture-a' AND item_id='third'",
            [],
        )
        .unwrap();
    assert_eq!(
        preview(&mut store, &["fixture-a"], "wrong_only").candidate_count,
        0
    );
    assert_eq!(
        store.start_exam(request("wrong_only")).err().unwrap().code,
        "NoCandidates"
    );
}

#[test]
fn empty_targets_fallback_and_count_errors_do_not_create_exams() {
    let (_temp, _paths, mut store) = setup();
    assert_eq!(
        store.start_exam(request("wrong_only")).err().unwrap().code,
        "NoCandidates"
    );
    let fallback = store.start_exam(request("wrong_first")).unwrap();
    assert_eq!(fallback.items.len(), 3);
    mark_third(&mut store);
    let mut too_many = request("wrong_only");
    too_many.count_mode = "count".into();
    too_many.count = Some(2);
    let before = store.exam_history("all").unwrap().len();
    assert_eq!(
        store.start_exam(too_many).err().unwrap().code,
        "InvalidCount"
    );
    assert_eq!(store.exam_history("all").unwrap().len(), before);
    for (ids, mode) in [
        (vec![], "general"),
        (vec!["missing".into()], "general"),
        (vec!["fixture-a".into(), "fixture-a".into()], "wrong_first"),
        (vec!["fixture-a".into()], "other"),
    ] {
        assert!(store
            .preview_candidates(CandidateRequest {
                dataset_ids: ids,
                review_mode: mode.into()
            })
            .is_err());
    }
}

#[test]
fn random_review_exams_are_unique_and_keep_wrong_priority() {
    let (_temp, _paths, mut store) = setup();
    mark_third(&mut store);
    for _ in 0..8 {
        let mut req = request("wrong_first");
        req.order_mode = "random".into();
        req.count_mode = "count".into();
        req.count = Some(2);
        let session = store.start_exam(req.clone()).unwrap();
        assert_eq!(session.items.len(), 2);
        assert!(session.items.iter().any(|i| i.item_id == "third"));
        assert_ne!(session.items[0].item_id, session.items[1].item_id);
        let retry = store.start_exam(req).unwrap();
        assert_eq!(
            session.items.iter().map(|i| &i.item_id).collect::<Vec<_>>(),
            retry.items.iter().map(|i| &i.item_id).collect::<Vec<_>>()
        );
    }
}

#[test]
fn history_filters_sort_and_do_not_expose_incomplete_scores_or_answers() {
    let (_temp, _paths, mut store) = setup();
    let old = mark_third(&mut store);
    let newer = mark_third(&mut store);
    store
        .connection
        .execute(
            "UPDATE exams SET completed_at=?1 WHERE exam_id=?2",
            params!["2300-01-01T00:00:00Z", old.exam_id],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE exams SET completed_at=?1 WHERE exam_id=?2",
            params!["1900-01-01T00:00:00Z", newer.exam_id],
        )
        .unwrap();
    let drafting = store.start_exam(request("general")).unwrap();
    let submitted = store.start_exam(request("general")).unwrap();
    store
        .submit_exam(SubmitExamRequest {
            exam_id: submitted.exam_id.clone(),
            revision: 1,
            current_position: 0,
            responses: vec!["\u{0085}".into(), "\u{feff}".into(), "".into()],
        })
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE exams SET saved_at='2400-01-01T00:00:00Z' WHERE exam_id=?1",
            [&drafting.exam_id],
        )
        .unwrap();
    let all = store.exam_history("all").unwrap();
    assert_eq!(all.len(), 4);
    assert_eq!(all[0].record().unwrap().exam_id, drafting.exam_id);
    assert_eq!(all[1].record().unwrap().exam_id, submitted.exam_id);
    assert_eq!(all[2].record().unwrap().exam_id, newer.exam_id);
    assert_eq!(all[3].record().unwrap().exam_id, old.exam_id);
    assert_eq!(all[0].record().unwrap().correct_count, None);
    assert_eq!(all[1].record().unwrap().correct_count, None);
    assert_eq!(all[1].record().unwrap().answered_count, 1);
    assert_eq!(all[2].record().unwrap().correct_count, Some(2));
    assert_eq!(store.exam_history("completed").unwrap().len(), 2);
    assert_eq!(store.exam_history("in_progress").unwrap().len(), 1);
    assert_eq!(store.exam_history("submitted").unwrap().len(), 1);
    let dto = serde_json::to_value(&all).unwrap();
    for row in dto.as_array().unwrap() {
        for key in ["question", "answer", "items", "responses", "response_text"] {
            assert!(row.get(key).is_none());
        }
    }
    assert_eq!(
        store.exam_history("unknown").err().unwrap().code,
        "ValidationError"
    );
    store.learner_id = Uuid::new_v4().to_string();
    assert!(store.exam_history("all").unwrap().is_empty());
}

#[test]
fn three_consecutive_correct_reset_on_wrong_and_old_result_stays_immutable() {
    let (_temp, paths, mut store) = setup();
    let old = mark_third(&mut store);
    let first = finish(&mut store, request("wrong_only"), &["C"]);
    assert_eq!(first.items[0].correct_streak, 1);
    assert!(first.items[0].is_review_target);
    let reset = finish(&mut store, request("wrong_only"), &[""]);
    assert_eq!(reset.items[0].correct_streak, 0);
    for expected in 1..=3 {
        let result = finish(&mut store, request("wrong_only"), &["C"]);
        assert_eq!(result.items[0].correct_streak, expected);
        assert_eq!(result.items[0].is_review_target, expected < 3);
    }
    assert_eq!(
        preview(&mut store, &["fixture-a"], "wrong_only").candidate_count,
        0
    );
    let before = store.exam_history("all").unwrap().len();
    for _ in 0..3 {
        let past = store.exam_result(&old.exam_id).unwrap();
        assert_eq!(past.correct_count, 2);
        assert_eq!(past.items[2].judgement, "incorrect");
        assert!(!past.items[2].is_review_target);
        assert_eq!(past.items[2].correct_streak, 3);
    }
    assert_eq!(store.exam_history("all").unwrap().len(), before);
    drop(store);
    let mut reopened = Store::open(&paths).unwrap();
    assert_eq!(reopened.exam_history("completed").unwrap().len(), before);
    let again = mark_third(&mut reopened);
    assert!(again.items[2].is_review_target);
    assert_eq!(again.items[2].correct_streak, 0);
}

#[test]
fn full_mode_and_snapshot_retry_survive_target_changes() {
    let (_temp, _paths, mut store) = setup();
    mark_third(&mut store);
    let req = request("wrong_only");
    let original = store.start_exam(req.clone()).unwrap();
    for _ in 0..3 {
        finish(&mut store, request("wrong_only"), &["C"]);
    }
    assert_eq!(
        preview(&mut store, &["fixture-a"], "wrong_only").candidate_count,
        0
    );
    let retry = store.start_exam(req).unwrap();
    assert_eq!(retry.items.len(), original.items.len());
    assert_eq!(retry.items[0].item_id, original.items[0].item_id);
    let full = store.start_exam(request("wrong_first")).unwrap();
    assert_eq!(full.items.len(), 3);
}

#[test]
fn damaged_exam_marks_only_its_row_and_keeps_other_history() {
    let (_temp, _paths, mut store) = setup();
    let completed = mark_third(&mut store);
    let broken_draft = store.start_exam(request("general")).unwrap();
    let healthy_draft = store.start_exam(request("general")).unwrap();
    let broken_result = mark_third(&mut store);
    store
        .connection
        .execute(
            "UPDATE exams SET dataset_titles_snapshot_json='damaged' WHERE exam_id=?1",
            [&broken_draft.exam_id],
        )
        .unwrap();
    store
        .connection
        .execute(
            "UPDATE exam_responses SET judgement='ungraded',incorrect_reason=NULL WHERE exam_id=?1 AND position=0",
            [&broken_result.exam_id],
        )
        .unwrap();

    let all = store.exam_history("all").unwrap();
    assert_eq!(all.len(), 4);
    assert_eq!(
        all.iter().filter(|entry| entry.record().is_none()).count(),
        2
    );
    assert_eq!(all[0].record().unwrap().exam_id, healthy_draft.exam_id);
    assert_eq!(all[1].record().unwrap().exam_id, completed.exam_id);
    assert_eq!(all[1].record().unwrap().correct_count, Some(2));
    assert!(all[2].record().is_none() && all[3].record().is_none());
    let pending = store.pending_exams().unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].record().unwrap().exam_id, healthy_draft.exam_id);
    assert!(pending[1].record().is_none());
    // 손상된 시험은 그대로 보존하고 다른 시험은 계속 열 수 있다.
    assert!(store.exam_session(&broken_draft.exam_id).is_err());
    assert!(store.exam_session(&healthy_draft.exam_id).is_ok());
}

#[test]
fn unreadable_basic_columns_are_isolated_sorted_last_and_do_not_invent_metadata() {
    for damage in [
        "completion_order='broken'",
        "saved_at=X'FF'",
        "saved_at='not-a-time'",
        "status='broken'",
        "exam_id=X'FF'",
    ] {
        let (_temp, _paths, mut store) = setup();
        let good = mark_third(&mut store);
        let bad = mark_third(&mut store);
        store
            .connection
            .execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;")
            .unwrap();
        store
            .connection
            .execute(
                &format!("UPDATE exams SET {damage} WHERE exam_id=?1"),
                [&bad.exam_id],
            )
            .unwrap();
        let rows = store.exam_history("all").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].record().unwrap().exam_id, good.exam_id);
        assert!(rows[1].record().is_none());
        let dto = serde_json::to_value(&rows[1]).unwrap();
        assert_eq!(dto["kind"], "error");
        for field in [
            "exam_id",
            "status",
            "question_count",
            "saved_at",
            "completed_at",
        ] {
            assert!(dto.get(field).is_none());
        }
        assert_eq!(
            serde_json::to_value(store.exam_history("all").unwrap()).unwrap(),
            serde_json::to_value(rows).unwrap()
        );
    }
}

#[test]
fn broken_pending_id_and_timestamp_keep_other_pending_and_dataset_lists_available() {
    for damage in ["exam_id=X'FF'", "saved_at=X'FF'"] {
        let (_temp, _paths, mut store) = setup();
        let bad = store.start_exam(request("general")).unwrap();
        let good = store.start_exam(request("general")).unwrap();
        store
            .connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .unwrap();
        store
            .connection
            .execute(
                &format!("UPDATE exams SET {damage} WHERE exam_id=?1"),
                [&bad.exam_id],
            )
            .unwrap();
        let pending = store.pending_exams().unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].record().unwrap().exam_id, good.exam_id);
        assert!(pending[1].record().is_none());
        assert!(!store.list_datasets().unwrap().is_empty());
    }
}
