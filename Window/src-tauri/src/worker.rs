use crate::exam::{CandidatePreview, CandidateRequest, ExamHistoryRow};
use crate::exam::{
    ExamResult, ExamSession, PendingExam, SaveProgressResult, StartExamRequest, SubmitExamRequest,
};
use crate::{
    error::{AppError, AppResult},
    storage::{AppPaths, AppStatus, DatasetSummary, Store},
};
use std::sync::mpsc::{self, Sender};

enum Job {
    Status(Sender<AppResult<AppStatus>>),
    Datasets(Sender<AppResult<Vec<DatasetSummary>>>),
    Start(StartExamRequest, Sender<AppResult<ExamSession>>),
    Session(String, Sender<AppResult<ExamSession>>),
    Submit(SubmitExamRequest, Sender<AppResult<ExamSession>>),
    Complete(String, Sender<AppResult<ExamSession>>),
    Result(String, Sender<AppResult<ExamResult>>),
    Save(SubmitExamRequest, Sender<AppResult<SaveProgressResult>>),
    Pending(Sender<AppResult<Vec<PendingExam>>>),
    Candidates(CandidateRequest, Sender<AppResult<CandidatePreview>>),
    History(String, Sender<AppResult<Vec<ExamHistoryRow>>>),
}

impl Job {
    fn fail(self, error: AppError) {
        match self {
            Self::Status(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Datasets(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Start(_, reply)
            | Self::Session(_, reply)
            | Self::Submit(_, reply)
            | Self::Complete(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Result(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Save(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Pending(reply) => {
                let _ = reply.send(Err(error));
            }
            Self::Candidates(_, reply) => {
                let _ = reply.send(Err(error));
            }
            Self::History(_, reply) => {
                let _ = reply.send(Err(error));
            }
        }
    }
}

#[derive(Clone)]
pub struct DbWorker {
    sender: Sender<Job>,
}

impl DbWorker {
    pub fn start(paths: AppPaths) -> AppResult<Self> {
        let (sender, receiver) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("studyapp-db".into())
            .spawn(move || {
                let mut store: Option<Store> = None;
                while let Ok(job) = receiver.recv() {
                    if store.is_none() {
                        match Store::open(&paths) {
                            Ok(opened) => store = Some(opened),
                            Err(error) => {
                                job.fail(error);
                                continue;
                            }
                        }
                    }
                    let current = store.as_mut().unwrap();
                    match job {
                        Job::Status(reply) => {
                            let _ = reply.send(current.status());
                        }
                        Job::Datasets(reply) => {
                            let _ = reply.send(current.list_datasets());
                        }
                        Job::Start(request, reply) => {
                            let _ = reply.send(current.start_exam(request));
                        }
                        Job::Session(id, reply) => {
                            let _ = reply.send(current.exam_session(&id));
                        }
                        Job::Submit(request, reply) => {
                            let _ = reply.send(current.submit_exam(request));
                        }
                        Job::Complete(id, reply) => {
                            let _ = reply.send(current.complete_exam(&id));
                        }
                        Job::Result(id, reply) => {
                            let _ = reply.send(current.exam_result(&id));
                        }
                        Job::Save(request, reply) => {
                            let _ = reply.send(current.save_progress(request));
                        }
                        Job::Pending(reply) => {
                            let _ = reply.send(current.pending_exams());
                        }
                        Job::Candidates(request, reply) => {
                            let _ = reply.send(current.preview_candidates(request));
                        }
                        Job::History(filter, reply) => {
                            let _ = reply.send(current.exam_history(&filter));
                        }
                    }
                }
            })
            .map_err(|_| {
                AppError::new("StorageFailure", "저장소 작업을 시작하지 못했어요.", true)
            })?;
        Ok(Self { sender })
    }

    pub fn status(&self) -> AppResult<AppStatus> {
        let (reply, receive) = mpsc::channel();
        self.sender
            .send(Job::Status(reply))
            .map_err(|_| disconnected())?;
        receive.recv().map_err(|_| disconnected())?
    }

    pub fn datasets(&self) -> AppResult<Vec<DatasetSummary>> {
        let (reply, receive) = mpsc::channel();
        self.sender
            .send(Job::Datasets(reply))
            .map_err(|_| disconnected())?;
        receive.recv().map_err(|_| disconnected())?
    }

    fn request<T>(&self, make_job: impl FnOnce(Sender<AppResult<T>>) -> Job) -> AppResult<T> {
        let (reply, receive) = mpsc::channel();
        self.sender
            .send(make_job(reply))
            .map_err(|_| disconnected())?;
        receive.recv().map_err(|_| disconnected())?
    }
    pub fn start_exam(&self, request: StartExamRequest) -> AppResult<ExamSession> {
        self.request(|reply| Job::Start(request, reply))
    }
    pub fn exam_session(&self, id: String) -> AppResult<ExamSession> {
        self.request(|reply| Job::Session(id, reply))
    }
    pub fn submit_exam(&self, request: SubmitExamRequest) -> AppResult<ExamSession> {
        self.request(|reply| Job::Submit(request, reply))
    }
    pub fn complete_exam(&self, id: String) -> AppResult<ExamSession> {
        self.request(|reply| Job::Complete(id, reply))
    }
    pub fn exam_result(&self, id: String) -> AppResult<ExamResult> {
        self.request(|reply| Job::Result(id, reply))
    }
    pub fn save_progress(&self, request: SubmitExamRequest) -> AppResult<SaveProgressResult> {
        self.request(|reply| Job::Save(request, reply))
    }
    pub fn pending_exams(&self) -> AppResult<Vec<PendingExam>> {
        self.request(Job::Pending)
    }
    pub fn preview_candidates(&self, request: CandidateRequest) -> AppResult<CandidatePreview> {
        self.request(|reply| Job::Candidates(request, reply))
    }
    pub fn exam_history(&self, filter: String) -> AppResult<Vec<ExamHistoryRow>> {
        self.request(|reply| Job::History(filter, reply))
    }
}

fn disconnected() -> AppError {
    AppError::new(
        "StorageFailure",
        "저장소 연결을 확인하지 못했어요. 앱을 다시 실행해주세요.",
        true,
    )
}
