pub type Result<T, E = AppError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("vault is locked")]
    VaultLocked,

    #[error("vault already initialized")]
    VaultAlreadyInitialized,

    #[error("incorrect master password")]
    BadPassword,

    #[error("cryptography error")]
    Crypto,

    #[error("not found")]
    NotFound,

    #[error("validation: {0}")]
    Validation(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("ssh: {0}")]
    Ssh(String),

    /// A forward's bind address is already taken — by another ezSsh tab,
    /// another ezSsh window/process, or (for remote) the same session
    /// already forwarding that port. Not a failure the user needs to fix:
    /// the surfacing layer renders it as a neutral "in use elsewhere"
    /// state with Start still available, not a red error.
    #[error("port conflict: {0}")]
    PortConflict(String),

    #[error("authentication failed")]
    AuthFailed,

    #[error("host key mismatch (expected {expected}, got {actual})")]
    HostKeyMismatch { expected: String, actual: String },

    /// No stored key for this host yet. `actual` is the SHA-256 fingerprint
    /// the server presented, for the user to verify before trusting it.
    #[error("host key not yet trusted")]
    HostKeyUntrusted { actual: String },

    #[error("sftp: {0}")]
    Sftp(String),

    #[error("path traversal rejected")]
    PathTraversal,

    #[error("VcXsrv is not installed")]
    XServerMissing,

    #[error("transfer cancelled")]
    TransferCancelled,

    #[error("updater: {0}")]
    Updater(String),
}

impl From<russh::Error> for AppError {
    fn from(e: russh::Error) -> Self {
        AppError::Ssh(e.to_string())
    }
}

impl From<russh_sftp::client::error::Error> for AppError {
    fn from(e: russh_sftp::client::error::Error) -> Self {
        AppError::Sftp(e.to_string())
    }
}
