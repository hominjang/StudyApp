use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
    pub field_path: Option<String>,
    pub retryable: bool,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            field_path: None,
            retryable,
        }
    }

    pub fn validation(field: impl Into<String>) -> Self {
        Self {
            code: "ValidationError",
            message: "학습 자료의 형식이나 연결 관계를 확인해주세요.".into(),
            field_path: Some(field.into()),
            retryable: false,
        }
    }

    pub fn inconsistent() -> Self {
        Self::new(
            "DataInconsistent",
            "저장된 자료의 상태를 확인하지 못했어요. 기존 자료는 유지됩니다.",
            false,
        )
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        let code = match error {
            rusqlite::Error::SqliteFailure(ref error, _)
                if matches!(
                    error.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                ) =>
            {
                "StorageBusy"
            }
            _ => "StorageFailure",
        };
        Self::new(
            code,
            "학습 자료를 저장하거나 조회하지 못했어요. 다시 시도해주세요.",
            true,
        )
    }
}
