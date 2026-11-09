//! The error-code registry. Codes are part of the wire contract: append,
//! never reuse. Clients branch on `code`; `message` is for people.

use vcsstar_types::ServiceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum Code {
    Internal = 1,
    Unavailable = 2,
    InvalidArgument = 3,
    Unauthenticated = 10,
    Forbidden = 11,
    TokenExpired = 12,
    NotFound = 20,
    AlreadyExists = 21,
    Conflict = 22,
    PreconditionFailed = 23,
    NotImplemented = 90,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct Error {
    pub code: Code,
    pub message: String,
    pub detail: Option<String>,
}

impl Error {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(Code::InvalidArgument, message)
    }
    pub fn not_found(what: &str) -> Self {
        Self::new(Code::NotFound, format!("{what} not found"))
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(Code::Forbidden, message)
    }
    pub fn unauthenticated() -> Self {
        Self::new(Code::Unauthenticated, "authentication required")
    }
    pub fn not_implemented(op: &str) -> Self {
        Self::new(Code::NotImplemented, format!("{op} is not implemented yet"))
    }
    pub fn internal(e: impl std::fmt::Display) -> Self {
        tracing::error!(error = %e, "internal error");
        Self::new(Code::Internal, "internal error")
    }
}

impl From<Error> for ServiceError {
    fn from(e: Error) -> Self {
        let message = match e.detail {
            Some(d) if !d.is_empty() => format!("{} ({d})", e.message),
            _ => e.message,
        };
        ServiceError {
            code: e.code as u64,
            message,
        }
    }
}

impl From<vcsstar_db::DbError> for Error {
    fn from(e: vcsstar_db::DbError) -> Self {
        match e {
            vcsstar_db::DbError::Pool(_) => {
                Error::new(Code::Unavailable, "database pool exhausted")
            }
            other => Error::internal(other),
        }
    }
}

impl From<diesel::result::Error> for Error {
    fn from(e: diesel::result::Error) -> Self {
        use diesel::result::{DatabaseErrorKind, Error as DE};
        match e {
            DE::NotFound => Error::new(Code::NotFound, "not found"),
            DE::DatabaseError(DatabaseErrorKind::UniqueViolation, info) => {
                let what = info
                    .table_name()
                    .map(|t| t.trim_end_matches('s').replace('_', " "))
                    .unwrap_or_else(|| "row".into());
                Error::new(Code::AlreadyExists, format!("{what} already exists"))
            }
            DE::DatabaseError(DatabaseErrorKind::ForeignKeyViolation, info) => {
                Error::new(Code::PreconditionFailed, "referenced row is missing")
                    .with_detail(info.message().to_string())
            }
            DE::DatabaseError(DatabaseErrorKind::CheckViolation, info) => {
                Error::new(Code::InvalidArgument, "value not allowed")
                    .with_detail(info.message().to_string())
            }
            other => Error::internal(other),
        }
    }
}
