use super::*;
use serde_json::json;
use std::path::Path;

// 실제 학습 자료를 포함하지 않는 가상 fixture다.
pub(super) fn fixture(root: &Path) -> AppPaths {
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("catalog.json"),
        serde_json::to_vec(&json!({
            "schema_version":"1.0", "datasets":[{"dataset_id":"fixture-a","title":"가상 자료",
            "learning_file":"learning.json","supplement_file":"supplement.json","question_count":1}]
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(source.join("learning.json"), serde_json::to_vec(&json!({
        "schema_version":"1.0","dataset_id":"fixture-a","title":"가상 자료",
        "items":[{"id":"fixture-item","question":"가상 질문","answer":"a  b","accepted_answers":["alias"]}]
    })).unwrap()).unwrap();
    fs::write(source.join("supplement.json"), serde_json::to_vec(&json!({
        "schema_version":"1.0","dataset_id":"fixture-a","dataset_metadata":{"note":"가상"},
        "items":[{"id":"fixture-item","explanation":"가상 해설","source_refs":[{"dataset_id":"원문 ID","item_id":"Original_ID"}]}]
    })).unwrap()).unwrap();
    AppPaths {
        db_dir: root.join("db"),
        initial_dir: source,
    }
}

fn mutate_learning(paths: &AppPaths, change: impl FnOnce(&mut serde_json::Value)) {
    let file = paths.initial_dir.join("learning.json");
    let mut value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    change(&mut value);
    fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
fn registers_once_and_reopens_without_source_files() {
    let temp = tempfile::tempdir().unwrap();
    let paths = fixture(temp.path());
    let mut first = Store::open(&paths).unwrap();
    let profile = first.learner_id.clone();
    assert_eq!(Uuid::parse_str(&profile).unwrap().get_version_num(), 4);
    assert_eq!(Uuid::parse_str(&profile).unwrap().to_string(), profile);
    assert_eq!(first.status().unwrap().question_count, 1);
    let (answer, aliases): (String, String) = first
        .connection
        .query_row("SELECT answer,accepted_answers_json FROM items", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(answer, "a  b");
    assert_eq!(aliases, "[\"alias\"]");
    let summary = serde_json::to_value(first.list_datasets().unwrap()).unwrap();
    assert!(summary[0].get("answer").is_none());
    assert!(summary[0].get("question").is_none());
    drop(first);
    let absent = AppPaths {
        initial_dir: temp.path().join("absent"),
        ..paths
    };
    let mut reopened = Store::open(&absent).unwrap();
    assert_eq!(reopened.learner_id, profile);
    assert_eq!(reopened.status().unwrap().dataset_count, 1);
    assert_eq!(
        reopened
            .connection
            .query_row("SELECT COUNT(*) FROM learners", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn invalid_ids_and_blank_answers_fail_before_final_db_creation() {
    for bad in ["Upper", "bad--id", "bad_id", "bad id"] {
        let temp = tempfile::tempdir().unwrap();
        let paths = fixture(temp.path());
        mutate_learning(&paths, |v| v["items"][0]["id"] = json!(bad));
        assert_eq!(Store::open(&paths).err().unwrap().code, "ValidationError");
        assert!(!paths.db_dir.join("study.db").exists());
    }
    for bad in ["", "\u{0085}", "\u{3000}", " answer", "answer "] {
        let temp = tempfile::tempdir().unwrap();
        let paths = fixture(temp.path());
        mutate_learning(&paths, |v| v["items"][0]["answer"] = json!(bad));
        assert!(Store::open(&paths).is_err());
        assert!(!paths.db_dir.join("study.db").exists());
    }
}

#[test]
fn invalid_dataset_duplicate_item_and_invalid_alias_are_rejected() {
    for kind in 0..3 {
        let temp = tempfile::tempdir().unwrap();
        let paths = fixture(temp.path());
        mutate_learning(&paths, |v| match kind {
            0 => v["dataset_id"] = json!("Invalid_ID"),
            1 => {
                let item = v["items"][0].clone();
                v["items"].as_array_mut().unwrap().push(item);
            }
            _ => v["items"][0]["accepted_answers"] = json!(["\u{0085}"]),
        });
        if kind == 1 {
            let file = paths.initial_dir.join("catalog.json");
            let mut catalog: serde_json::Value =
                serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
            catalog["datasets"][0]["question_count"] = json!(2);
            fs::write(file, serde_json::to_vec(&catalog).unwrap()).unwrap();
        }
        assert!(Store::open(&paths).is_err());
        assert!(!paths.db_dir.join("study.db").exists());
    }
}

#[test]
fn source_refs_are_preserved_without_runtime_source_or_slug_checks() {
    let temp = tempfile::tempdir().unwrap();
    let paths = fixture(temp.path());
    let store = Store::open(&paths).unwrap();
    let payload: String = store
        .connection
        .query_row("SELECT payload_json FROM supplements", [], |r| r.get(0))
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["source_refs"][0]["item_id"], "Original_ID");
}

#[test]
fn corrupt_existing_db_is_not_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let paths = fixture(temp.path());
    fs::create_dir_all(&paths.db_dir).unwrap();
    let file = paths.db_dir.join("study.db");
    let before = b"not a sqlite database";
    fs::write(&file, before).unwrap();
    assert!(Store::open(&paths).is_err());
    assert_eq!(fs::read(&file).unwrap(), before);
}

#[test]
fn unsupported_db_version_is_not_reinitialized() {
    let temp = tempfile::tempdir().unwrap();
    let paths = fixture(temp.path());
    let store = Store::open(&paths).unwrap();
    let learner = store.learner_id.clone();
    store
        .connection
        .execute(
            "UPDATE app_metadata SET value='99' WHERE key='db_schema_version'",
            [],
        )
        .unwrap();
    drop(store);
    assert_eq!(
        Store::open(&paths).err().unwrap().code,
        "UnsupportedDbVersion"
    );
    let db = Connection::open(paths.db_dir.join("study.db")).unwrap();
    assert_eq!(metadata(&db, "local_learner_id").unwrap().unwrap(), learner);
    assert_eq!(metadata(&db, "db_schema_version").unwrap().unwrap(), "99");
}

#[test]
fn database_transaction_failure_rolls_back_schema_and_rows() {
    let temp = tempfile::tempdir().unwrap();
    let paths = fixture(temp.path());
    let packages = load_initial(&paths.initial_dir).unwrap();
    let mut db = Connection::open(temp.path().join("rollback.db")).unwrap();
    configure(&db).unwrap();
    let duplicate = vec![packages[0].clone(), packages[0].clone()];
    assert!(seed(&mut db, &duplicate, true).is_err());
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn missing_install_marker_with_existing_rows_is_rejected_without_source() {
    let temp = tempfile::tempdir().unwrap();
    let mut paths = fixture(temp.path());
    let store = Store::open(&paths).unwrap();
    let profile = store.learner_id.clone();
    store
        .connection
        .execute(
            "DELETE FROM app_metadata WHERE key='initial_data_installed'",
            [],
        )
        .unwrap();
    drop(store);
    paths.initial_dir = temp.path().join("absent");
    assert_eq!(Store::open(&paths).err().unwrap().code, "DataInconsistent");
    let db = Connection::open(paths.db_dir.join("study.db")).unwrap();
    assert_eq!(metadata(&db, "local_learner_id").unwrap().unwrap(), profile);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM items", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn known_empty_database_can_finish_initialization() {
    let temp = tempfile::tempdir().unwrap();
    let paths = fixture(temp.path());
    fs::create_dir_all(&paths.db_dir).unwrap();
    let db = Connection::open(paths.db_dir.join("study.db")).unwrap();
    db.execute_batch(SCHEMA).unwrap();
    db.execute("INSERT INTO app_metadata VALUES ('application_id','StudyApp'),('db_schema_version','1'),('initial_data_installed','0')", []).unwrap();
    drop(db);
    assert_eq!(
        Store::open(&paths)
            .unwrap()
            .status()
            .unwrap()
            .question_count,
        1
    );
}

#[test]
fn catalog_path_escape_and_count_mismatch_are_rejected() {
    for mode in 0..2 {
        let temp = tempfile::tempdir().unwrap();
        let paths = fixture(temp.path());
        let file = paths.initial_dir.join("catalog.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        if mode == 0 {
            value["datasets"][0]["learning_file"] = json!("../learning.json");
        } else {
            value["datasets"][0]["question_count"] = json!(99);
        }
        fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(Store::open(&paths).is_err());
        assert!(!paths.db_dir.join("study.db").exists());
    }
}

#[test]
fn worker_retries_initialization_after_data_is_prepared() {
    let temp = tempfile::tempdir().unwrap();
    let paths = fixture(temp.path());
    let expected = paths.initial_dir.join("catalog.json");
    let saved = fs::read(&expected).unwrap();
    fs::write(&expected, b"invalid").unwrap();
    let worker = crate::worker::DbWorker::start(paths).unwrap();
    assert_eq!(worker.status().err().unwrap().code, "ValidationError");
    fs::write(expected, saved).unwrap();
    assert_eq!(worker.status().unwrap().question_count, 1);
    assert_eq!(worker.datasets().unwrap().len(), 1);
}

#[test]
#[ignore = "실제 자료는 Git 제외. STUDYAPP_TEST_INITIAL_DIR를 준비한 환경에서 별도 실행"]
fn private_local_data_roundtrip() {
    let source =
        std::env::var_os("STUDYAPP_TEST_INITIAL_DIR").expect("local dataset directory required");
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths {
        db_dir: temp.path().join("db"),
        initial_dir: PathBuf::from(source),
    };
    let mut store = Store::open(&paths).unwrap();
    assert_eq!(store.status().unwrap().dataset_count, 2);
    assert_eq!(store.status().unwrap().question_count, 254);
    let profile = store.learner_id.clone();
    drop(store);
    let mut reopened = Store::open(&paths).unwrap();
    assert_eq!(reopened.learner_id, profile);
    assert_eq!(reopened.status().unwrap().question_count, 254);
}
