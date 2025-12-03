use std::fmt::Display;

#[derive(Debug)]
pub struct AgendaError(pub(crate) ErrorKind);

#[derive(Debug)]
pub(crate) enum ErrorKind {
    SerializationError { message: String },
    DatabaseError { message: String },
    CronExpFailed { cron: String },
    JobNotFound,
}

impl Display for AgendaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            ErrorKind::CronExpFailed { cron } => {
                writeln!(f, "Failed to parse cron expression: {cron}")
            }
            ErrorKind::DatabaseError { message } => writeln!(f, "Database Error: {message}"),
            ErrorKind::SerializationError { message } => writeln!(
                f,
                "Serialization error, maybe you assigned an invalid value to a job? {message}"
            ),
            ErrorKind::JobNotFound => writeln!(
                f,
                "Tried to run a job that is not in the agenda. Please register your job before access it"
            ),
        }
    }
}

impl From<serde_json::Error> for AgendaError {
    fn from(value: serde_json::Error) -> Self {
        AgendaError(ErrorKind::SerializationError {
            message: value.to_string(),
        })
    }
}

impl From<sqlx::Error> for AgendaError {
    fn from(value: sqlx::Error) -> Self {
        AgendaError(ErrorKind::DatabaseError {
            message: value.to_string(),
        })
    }
}

impl From<chrono_tz::ParseError> for AgendaError {
    fn from(value: chrono_tz::ParseError) -> Self {
        AgendaError(ErrorKind::DatabaseError {
            message: value.to_string(),
        })
    }
}

impl From<cron::error::Error> for AgendaError {
    fn from(value: cron::error::Error) -> Self {
        AgendaError(ErrorKind::CronExpFailed {
            cron: value.to_string(),
        })
    }
}

impl std::error::Error for AgendaError {}
