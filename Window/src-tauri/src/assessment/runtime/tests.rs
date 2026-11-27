use super::*;
use crate::{
    assessment::{AddReviewRequest, Judgement, SetAssessmentRequest},
    storage::AppPaths,
    worker::DbWorker,
};
use serde_json::json;
use std::fs;

fn fixture() -> (tempfile::TempDir, AppPaths, AssessmentStore) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("initial");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("catalog.json"),serde_json::to_vec(&json!({"schema_version":"2.0","datasets":[{"dataset_id":"synthetic","title":"Synthetic","learning_file":"learning.json","supplement_file":"supplement.json","question_count":3}]})).unwrap()).unwrap();
    fs::write(source.join("learning.json"),serde_json::to_vec(&json!({"schema_version":"2.0","dataset_id":"synthetic","title":"Synthetic","items":[
        {"id":"short","type":"short_answer","question":"Synthetic short question","answer":"alpha","accepted_answers":["alias"]},
        {"id":"explain","type":"descriptive","question":"Synthetic description","reference_answer":"Synthetic reference with more than four words\r\nsecond line","key_points":["Synthetic key point"]},
        {"id":"basic","type":"short_answer","question":"Synthetic basic question","answer":"base"}]})).unwrap()).unwrap();
    fs::write(source.join("supplement.json"),serde_json::to_vec(&json!({"schema_version":"2.0","dataset_id":"synthetic","items":[],"learning_links":[{"item_id":"explain","prerequisites":[{"dataset_id":"synthetic","item_id":"basic"},{"dataset_id":"missing","item_id":"unknown"}]}]})).unwrap()).unwrap();
    let paths = AppPaths {
        db_dir: temp.path().join("db"),
        initial_dir: source,
    };
    let store = AssessmentStore::initialize(&paths).unwrap();
    (temp, paths, store)
}
fn request() -> StartExamRequest {
    StartExamRequest {
        exam_id: Uuid::new_v4().to_string(),
        dataset_ids: vec!["synthetic".into()],
        count_mode: "all".into(),
        count: None,
        order_mode: "ordered".into(),
        review_mode: "general".into(),
    }
}
fn submit(id: &str, answers: &[&str]) -> SubmitExamRequest {
    SubmitExamRequest {
        exam_id: id.into(),
        revision: 1,
        current_position: 0,
        responses: answers.iter().map(|s| s.to_string()).collect(),
    }
}
fn finish(s: &mut AssessmentStore, answers: &[&str]) -> String {
    let session = s.start_exam(request()).unwrap();
    s.submit_exam(submit(&session.exam_id, answers)).unwrap();
    s.complete_exam(&session.exam_id).unwrap();
    session.exam_id
}
fn evaluate(id: &str, judgement: Judgement) -> SetAssessmentRequest {
    SetAssessmentRequest {
        exam_id: id.into(),
        position: 1,
        expected_revision: 0,
        judgement,
        note: None,
        request_id: Uuid::new_v4().to_string(),
    }
}
fn count(s: &AssessmentStore, table: &str) -> i64 {
    s.db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn fresh_store_registers_mixed_data_once_and_draft_never_exposes_criteria() {
    let (_temp, paths, mut s) = fixture();
    let r = request();
    let a = s.start_exam(r.clone()).unwrap();
    assert_eq!(a.db_version, 2);
    assert_eq!(a.items[1].item_type, "descriptive");
    let dto = serde_json::to_value(&a).unwrap();
    for item in dto["items"].as_array().unwrap() {
        for key in [
            "answer",
            "accepted_answers",
            "reference_answer",
            "key_points",
            "criteria",
        ] {
            assert!(item.get(key).is_none());
        }
    }
    assert_eq!(s.start_exam(r).unwrap().exam_id, a.exam_id);
    assert_eq!(count(&s, "exams"), 1);
    drop(s);
    fs::write(
        paths.initial_dir.join("learning.json"),
        b"invalid synthetic input",
    )
    .unwrap();
    let mut reopened = AssessmentStore::initialize(&paths).unwrap();
    assert_eq!(reopened.status().unwrap().question_count, 3);
    assert_eq!(reopened.exam_session(&a.exam_id).unwrap().items.len(), 3);
}

#[test]
fn actual_mixed_exam_roundtrip_restores_raw_answers_and_supports_late_partial_assessment() {
    let (_temp, paths, mut s) = fixture();
    let session = s.start_exam(request()).unwrap();
    let req = submit(&session.exam_id, &["alias", "Original\r\nresponse", ""]);
    s.save_progress(req.clone()).unwrap();
    drop(s);
    let mut s = AssessmentStore::open(&paths.db_dir.join("study.db")).unwrap();
    assert_eq!(
        s.exam_session(&session.exam_id).unwrap().items[1].response_text,
        "Original\r\nresponse"
    );
    s.submit_exam(req).unwrap();
    assert_eq!(
        s.assessment_view(&session.exam_id).err().unwrap().code,
        "InvalidState"
    );
    s.complete_exam(&session.exam_id).unwrap();
    let view = s.assessment_view(&session.exam_id).unwrap();
    assert_eq!(
        view.counts,
        crate::assessment::AssessmentCounts {
            correct: 1,
            partial: 0,
            incorrect: 1,
            ungraded: 1
        }
    );
    assert_eq!(
        view.items[2].incorrect_reason.as_deref(),
        Some("unanswered")
    );
    s.set_assessment(evaluate(&session.exam_id, Judgement::Partial))
        .unwrap();
    let view = s.assessment_view(&session.exam_id).unwrap();
    assert_eq!(view.counts.partial, 1);
    assert_eq!(view.assessment_status, "complete");
    assert!(view.items[1].is_review_target);
    assert_eq!(view.items[1].response_text, "Original\r\nresponse");
}

#[test]
fn v2_save_protocol_rejects_conflicts_stale_requests_and_post_submission_overwrites() {
    let (_temp, _paths, mut s) = fixture();
    let session = s.start_exam(request()).unwrap();
    let mut r = submit(&session.exam_id, &["alpha", "Synthetic response", "base"]);
    r.revision = 2;
    s.save_progress(r.clone()).unwrap();
    let mut old = r.clone();
    old.revision = 1;
    assert_eq!(s.save_progress(old).unwrap().outcome, "stale");
    let mut conflict = r.clone();
    conflict.responses[0] = "different".into();
    assert_eq!(
        s.save_progress(conflict).err().unwrap().code,
        "RevisionConflict"
    );
    s.submit_exam(r.clone()).unwrap();
    r.revision = 10;
    r.responses[0] = "late".into();
    assert_eq!(s.save_progress(r).unwrap().outcome, "locked");
    s.complete_exam(&session.exam_id).unwrap();
    assert_eq!(
        s.assessment_view(&session.exam_id).unwrap().items[0].response_text,
        "alpha"
    );
}

#[test]
fn completion_is_atomic_and_retry_does_not_duplicate_or_undo_manual_assessment() {
    let (_temp, _paths, mut s) = fixture();
    let session = s.start_exam(request()).unwrap();
    s.submit_exam(submit(
        &session.exam_id,
        &["alpha", "Synthetic answer", "base"],
    ))
    .unwrap();
    s.db.execute_batch("CREATE TRIGGER fail_event BEFORE INSERT ON learning_events BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(s.complete_exam(&session.exam_id).is_err());
    assert_eq!(
        s.exam_session(&session.exam_id).unwrap().status,
        "submitted"
    );
    assert_eq!(count(&s, "assessment_events"), 0);
    assert_eq!(count(&s, "item_progress"), 0);
    s.db.execute_batch("DROP TRIGGER fail_event;").unwrap();
    s.complete_exam(&session.exam_id).unwrap();
    s.set_assessment(evaluate(&session.exam_id, Judgement::Correct))
        .unwrap();
    let before = count(&s, "assessment_events");
    s.complete_exam(&session.exam_id).unwrap();
    assert_eq!(count(&s, "assessment_events"), before);
    assert_eq!(count(&s, "learning_events"), 1);
    assert_eq!(
        s.assessment_view(&session.exam_id).unwrap().counts.correct,
        3
    );
}

#[test]
fn snapshot_criteria_survive_source_edits_and_descriptive_unanswered_uses_fixed_space_set() {
    let (_temp, _paths, mut s) = fixture();
    let session = s.start_exam(request()).unwrap();
    let changed = json(&Criteria::ShortAnswer {
        answer: "changed".into(),
        accepted_answers: vec![],
    })
    .unwrap();
    s.db.execute(
        "UPDATE items SET criteria_json=?1 WHERE item_id='short'",
        [changed],
    )
    .unwrap();
    s.submit_exam(submit(
        &session.exam_id,
        &["alpha", "\u{0085}\u{3000}", "base"],
    ))
    .unwrap();
    s.complete_exam(&session.exam_id).unwrap();
    let view = s.assessment_view(&session.exam_id).unwrap();
    assert_eq!(view.counts.correct, 2);
    assert_eq!(
        view.items[1].incorrect_reason.as_deref(),
        Some("unanswered")
    );
    let id = finish(&mut s, &["changed", "\u{feff}", "base"]);
    assert_eq!(s.assessment_view(&id).unwrap().counts.ungraded, 1);
}

#[test]
fn manual_basic_target_enters_existing_wrong_only_and_three_correct_attempts_clear_it() {
    let (_temp, _paths, mut s) = fixture();
    s.add_review(AddReviewRequest {
        dataset_id: "synthetic".into(),
        item_id: "basic".into(),
        expected_progress_revision: 0,
        request_id: Uuid::new_v4().to_string(),
    })
    .unwrap();
    for _ in 0..3 {
        let mut r = request();
        r.review_mode = "wrong_only".into();
        let session = s.start_exam(r).unwrap();
        assert_eq!(session.items.len(), 1);
        assert_eq!(session.items[0].item_id, "basic");
        s.submit_exam(submit(&session.exam_id, &["base"])).unwrap();
        s.complete_exam(&session.exam_id).unwrap();
    }
    assert_eq!(s.list_datasets().unwrap()[0].review_target_count, 0);
    let mut r = request();
    r.review_mode = "wrong_only".into();
    assert_eq!(s.start_exam(r).err().unwrap().code, "NoCandidates");
}

#[test]
fn history_tracks_evaluation_waiting_and_rechecks_latest_counts_without_score_leak() {
    let (_temp, _paths, mut s) = fixture();
    let draft = s.start_exam(request()).unwrap();
    let id = finish(&mut s, &["alpha", "Synthetic answer", "base"]);
    let all = s.exam_history("all").unwrap();
    assert_eq!(all[0].record().unwrap().exam_id, draft.exam_id);
    assert!(all[0].record().unwrap().assessment_counts.is_none());
    assert_eq!(s.exam_history("assessment_pending").unwrap().len(), 1);
    s.set_assessment(evaluate(&id, Judgement::Correct)).unwrap();
    assert!(s.exam_history("assessment_pending").unwrap().is_empty());
    assert_eq!(
        s.exam_history("completed").unwrap()[0]
            .record()
            .unwrap()
            .assessment_counts
            .as_ref()
            .unwrap()
            .correct,
        3
    );
}

#[test]
fn invalid_fresh_package_does_not_publish_database_and_worker_uses_v2_for_new_installations() {
    let (_temp, paths, s) = fixture();
    drop(s);
    let new = AppPaths {
        db_dir: paths.db_dir.parent().unwrap().join("fresh"),
        initial_dir: paths.initial_dir.clone(),
    };
    fs::write(paths.initial_dir.join("learning.json"), b"invalid").unwrap();
    assert!(AssessmentStore::initialize(&new).is_err());
    assert!(!new.db_dir.join("study.db").exists());
    let (_other, paths, s) = fixture();
    drop(s);
    let worker = DbWorker::start(paths).unwrap();
    assert_eq!(worker.status().unwrap().question_count, 3);
    let session = worker.start_exam(request()).unwrap();
    assert_eq!(session.db_version, 2);
    worker
        .submit_exam(submit(
            &session.exam_id,
            &["alpha", "Synthetic answer", "base"],
        ))
        .unwrap();
    worker.complete_exam(session.exam_id.clone()).unwrap();
    assert_eq!(
        worker
            .assessment_view(session.exam_id)
            .unwrap()
            .counts
            .ungraded,
        1
    );
}

#[test]
fn migrated_v1_pending_exam_finishes_in_v2_and_original_database_is_preserved() {
    let (temp, paths, mut legacy) = crate::migration::tests::setup();
    let id = crate::migration::tests::attempt(&mut legacy, "yes", "submitted");
    drop(legacy);
    let source = paths.db_dir.join("study.db");
    let bytes = fs::read(&source).unwrap();
    let copy = temp.path().join("copy.db");
    crate::migration::prepare_v2_copy(&source, &copy).unwrap();
    let mut s = AssessmentStore::open(&copy).unwrap();
    s.complete_exam(&id).unwrap();
    assert_eq!(s.assessment_view(&id).unwrap().counts.correct, 1);
    assert_eq!(fs::read(&source).unwrap(), bytes);
    let worker = DbWorker::start(paths).unwrap();
    assert_eq!(worker.exam_session(id).unwrap().db_version, 1);
}

#[test]
fn foundation_lookup_is_submission_gated_and_only_returns_questions_and_review_state() {
    let (_temp, _paths, mut s) = fixture();
    let session = s.start_exam(request()).unwrap();
    assert_eq!(
        s.foundation_links(&session.exam_id, 1).err().unwrap().code,
        "InvalidState"
    );
    s.submit_exam(submit(
        &session.exam_id,
        &["alpha", "Synthetic explanation", "base"],
    ))
    .unwrap();
    s.complete_exam(&session.exam_id).unwrap();
    let links = s.foundation_links(&session.exam_id, 1).unwrap();
    assert_eq!(links.len(), 2);
    assert!(links[0].issue.is_none());
    assert_eq!(links[1].issue.as_deref(), Some("Missing"));
    let dto = serde_json::to_value(links).unwrap();
    for key in ["answer", "criteria", "reference_answer", "key_points"] {
        assert!(dto[0].get(key).is_none());
    }
    s.db.execute("UPDATE items SET is_active=0 WHERE item_id='basic'", [])
        .unwrap();
    assert_eq!(
        s.foundation_links(&session.exam_id, 1).unwrap()[0]
            .issue
            .as_deref(),
        Some("Missing")
    );
}

#[test]
fn corrupt_assessment_chain_marks_only_affected_v2_history_row() {
    let (_temp, _paths, mut s) = fixture();
    let bad = finish(&mut s, &["alpha", "Synthetic explanation", "base"]);
    let good = finish(&mut s, &["alpha", "Synthetic explanation", "base"]);
    s.db.execute(
        "UPDATE response_assessments SET judgement='partial' WHERE exam_id=?1 AND position=0",
        [bad],
    )
    .unwrap();
    let rows = s.exam_history("completed").unwrap();
    assert_eq!(rows[0].record().unwrap().exam_id, good);
    assert!(rows[1].record().is_none());
}

#[test]
fn package_hash_includes_supplement_and_learning_links() {
    let (_temp, paths, s) = fixture();
    let before: String =
        s.db.query_row("SELECT content_hash FROM datasets", [], |r| r.get(0))
            .unwrap();
    let file = paths.initial_dir.join("supplement.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    value["dataset_metadata"] = serde_json::json!({"synthetic_note":"changed"});
    fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
    let next = AppPaths {
        db_dir: paths.db_dir.parent().unwrap().join("fresh-with-supplement"),
        initial_dir: paths.initial_dir,
    };
    let new = AssessmentStore::initialize(&next).unwrap();
    let after: String = new
        .db
        .query_row("SELECT content_hash FROM datasets", [], |r| r.get(0))
        .unwrap();
    assert_ne!(before, after);
}

#[test]
fn unknown_runtime_version_is_rejected_without_writing_the_existing_file() {
    let (_temp, paths, s) = fixture();
    s.db.execute(
        "UPDATE app_metadata SET value='99' WHERE key='db_schema_version'",
        [],
    )
    .unwrap();
    drop(s);
    let file = paths.db_dir.join("study.db");
    let before = fs::read(&file).unwrap();
    assert_eq!(
        crate::runtime_store::RuntimeStore::open(&paths)
            .err()
            .unwrap()
            .code,
        "UnsupportedDbVersion"
    );
    assert_eq!(fs::read(file).unwrap(), before);
}

#[test]
fn incomplete_v2_metadata_is_not_reinitialized_or_opened_as_ready() {
    let (_temp, paths, s) = fixture();
    s.db.execute(
        "UPDATE app_metadata SET value='0' WHERE key='initial_data_installed'",
        [],
    )
    .unwrap();
    drop(s);
    let file = paths.db_dir.join("study.db");
    let before = fs::read(&file).unwrap();
    assert_eq!(
        AssessmentStore::initialize(&paths).err().unwrap().code,
        "DataInconsistent"
    );
    assert_eq!(fs::read(file).unwrap(), before);
}
