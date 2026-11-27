pub mod assessment;
mod error;
mod exam;
mod import_file;
pub mod learning_format;
pub mod migration;
mod package;
mod runtime_store;
mod storage;
mod study_material;
mod transfer;
mod validation;
mod worker;

use error::{AppError, AppResult};
use exam::{CandidatePreview, CandidateRequest, ExamHistoryRow, RecordEntry};
use exam::{
    ExamResult, ExamSession, PendingExam, SaveProgressResult, StartExamRequest, SubmitExamRequest,
};
use std::path::PathBuf;
use storage::{AppPaths, AppStatus, DatasetSummary};
use tauri::Manager;
use worker::DbWorker;

#[tauri::command]
async fn update_study_material(
    state: tauri::State<'_, DbWorker>,
    request: transfer::EditMaterialRequest,
) -> AppResult<transfer::EditMaterialResult> {
    let worker = state.inner().clone();
    run_db(move || worker.edit_material(request)).await
}

#[tauri::command]
async fn browse_study_materials(
    state: tauri::State<'_, DbWorker>,
    request: study_material::StudyRequest,
) -> AppResult<study_material::StudyPage> {
    let worker = state.inner().clone();
    run_db(move || worker.study_materials(request)).await
}

#[tauri::command]
async fn choose_import_file(
    window: tauri::WebviewWindow,
    kind: String,
) -> AppResult<Option<import_file::SelectedImportFile>> {
    if !matches!(kind.as_str(), "learning" | "supplement") {
        return Err(AppError::validation("file_kind"));
    }
    #[cfg(windows)]
    {
        let owner = window.hwnd().map_err(|_| AppError::inconsistent())?.0 as isize;
        run_db(move || {
            std::thread::spawn(move || import_file::select(owner, &kind))
                .join()
                .map_err(|_| {
                    AppError::new(
                        "FileReadFailure",
                        "자료 선택을 완료하지 못했어요. 다시 시도해주세요.",
                        true,
                    )
                })?
        })
        .await
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err(AppError::new(
            "UnsupportedPlatform",
            "Windows 앱에서 파일을 선택해주세요.",
            false,
        ))
    }
}

#[tauri::command]
async fn preview_import(
    state: tauri::State<'_, DbWorker>,
    request: transfer::ImportRequest,
) -> AppResult<transfer::ImportPreview> {
    let worker = state.inner().clone();
    run_db(move || worker.preview_import(request)).await
}
#[tauri::command]
async fn apply_import(
    state: tauri::State<'_, DbWorker>,
    token: String,
) -> AppResult<transfer::ImportResult> {
    let worker = state.inner().clone();
    run_db(move || worker.apply_import(token)).await
}

use assessment::{
    AddReviewRequest, AssessmentView, FoundationLink, MutationResult, SetAssessmentRequest,
};
#[tauri::command]
async fn get_foundation_links(
    state: tauri::State<'_, DbWorker>,
    exam_id: String,
    position: u32,
) -> AppResult<Vec<FoundationLink>> {
    let worker = state.inner().clone();
    run_db(move || worker.foundation_links(exam_id, position)).await
}
#[tauri::command]
async fn get_assessment_view(
    state: tauri::State<'_, DbWorker>,
    exam_id: String,
) -> AppResult<AssessmentView> {
    let worker = state.inner().clone();
    run_db(move || worker.assessment_view(exam_id)).await
}
#[tauri::command]
async fn set_response_assessment(
    state: tauri::State<'_, DbWorker>,
    request: SetAssessmentRequest,
) -> AppResult<MutationResult> {
    let worker = state.inner().clone();
    run_db(move || worker.set_assessment(request)).await
}
#[tauri::command]
async fn add_review_target(
    state: tauri::State<'_, DbWorker>,
    request: AddReviewRequest,
) -> AppResult<MutationResult> {
    let worker = state.inner().clone();
    run_db(move || worker.add_review(request)).await
}

#[tauri::command]
async fn preview_candidates(
    state: tauri::State<'_, DbWorker>,
    request: CandidateRequest,
) -> AppResult<CandidatePreview> {
    let worker = state.inner().clone();
    run_db(move || worker.preview_candidates(request)).await
}
#[tauri::command]
async fn list_exam_history(
    state: tauri::State<'_, DbWorker>,
    filter: String,
) -> AppResult<Vec<RecordEntry<ExamHistoryRow>>> {
    let worker = state.inner().clone();
    run_db(move || worker.exam_history(filter)).await
}

async fn run_db<T: Send + 'static>(
    job: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(job)
        .await
        .map_err(|_| {
            AppError::new(
                "StorageFailure",
                "시험 처리 상태를 확인하지 못했어요. 같은 시험으로 다시 시도해주세요.",
                true,
            )
        })?
}

#[tauri::command]
async fn start_exam(
    state: tauri::State<'_, DbWorker>,
    request: StartExamRequest,
) -> AppResult<ExamSession> {
    let worker = state.inner().clone();
    run_db(move || worker.start_exam(request)).await
}
#[tauri::command]
async fn get_exam_session(
    state: tauri::State<'_, DbWorker>,
    exam_id: String,
) -> AppResult<ExamSession> {
    let worker = state.inner().clone();
    run_db(move || worker.exam_session(exam_id)).await
}
#[tauri::command]
async fn submit_exam(
    state: tauri::State<'_, DbWorker>,
    request: SubmitExamRequest,
) -> AppResult<ExamSession> {
    let worker = state.inner().clone();
    run_db(move || worker.submit_exam(request)).await
}
#[tauri::command]
async fn complete_exam(
    state: tauri::State<'_, DbWorker>,
    exam_id: String,
) -> AppResult<ExamSession> {
    let worker = state.inner().clone();
    run_db(move || worker.complete_exam(exam_id)).await
}
#[tauri::command]
async fn get_exam_result(
    state: tauri::State<'_, DbWorker>,
    exam_id: String,
) -> AppResult<ExamResult> {
    let worker = state.inner().clone();
    run_db(move || worker.exam_result(exam_id)).await
}

#[tauri::command]
async fn save_progress(
    state: tauri::State<'_, DbWorker>,
    request: SubmitExamRequest,
) -> AppResult<SaveProgressResult> {
    let worker = state.inner().clone();
    run_db(move || worker.save_progress(request)).await
}
#[tauri::command]
async fn list_pending_exams(
    state: tauri::State<'_, DbWorker>,
) -> AppResult<Vec<RecordEntry<PendingExam>>> {
    let worker = state.inner().clone();
    run_db(move || worker.pending_exams()).await
}

#[tauri::command]
async fn get_app_status(state: tauri::State<'_, DbWorker>) -> AppResult<AppStatus> {
    let worker = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || worker.status())
        .await
        .map_err(|_| AppError::new("StorageFailure", "저장소 상태를 확인하지 못했어요.", true))?
}

#[tauri::command]
async fn list_datasets(state: tauri::State<'_, DbWorker>) -> AppResult<Vec<DatasetSummary>> {
    let worker = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || worker.datasets())
        .await
        .map_err(|_| AppError::new("StorageFailure", "학습 자료를 불러오지 못했어요.", true))?
}

fn app_paths(app: &tauri::App) -> AppResult<AppPaths> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| {
            AppError::new(
                "StorageFailure",
                "사용자 저장 위치를 확인하지 못했어요.",
                false,
            )
        })?;
    let mut db_dir = base.join("StudyApp");
    let mut initial_dir = app
        .path()
        .resource_dir()
        .map_err(|_| {
            AppError::new(
                "DataUnavailable",
                "앱 자료 위치를 확인하지 못했어요.",
                false,
            )
        })?
        .join("initial-data");
    if cfg!(debug_assertions) {
        db_dir = std::env::var_os("STUDYAPP_DB_DIR")
            .map(PathBuf::from)
            .unwrap_or(db_dir);
        initial_dir = std::env::var_os("STUDYAPP_INITIAL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/initial-data")
            });
    }
    Ok(AppPaths {
        db_dir,
        initial_dir,
    })
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            app.manage(DbWorker::start(app_paths(app)?)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            update_study_material,
            browse_study_materials,
            choose_import_file,
            preview_import,
            apply_import,
            get_foundation_links,
            get_assessment_view,
            set_response_assessment,
            add_review_target,
            get_app_status,
            list_datasets,
            start_exam,
            get_exam_session,
            submit_exam,
            complete_exam,
            get_exam_result,
            save_progress,
            list_pending_exams,
            preview_candidates,
            list_exam_history
        ])
        .run(tauri::generate_context!())
        .expect("StudyApp 실행에 실패했습니다.");
}
