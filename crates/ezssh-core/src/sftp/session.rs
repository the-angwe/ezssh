use russh_sftp::client::SftpSession;

use crate::error::{AppError, Result};

/// One SFTP channel per SSH connection. russh-sftp multiplexes requests
/// by id and every verb takes `&self`, so a directory listing or a rename
/// from the file pane proceeds while a transfer streams on the same
/// session; only `File` handles are per caller. (An earlier version held
/// a mutex for the whole transfer, which froze the pane until the upload
/// finished.)
pub struct SftpHandle {
    session: SftpSession,
}

impl SftpHandle {
    pub fn new(session: SftpSession) -> Self {
        Self { session }
    }

    /// Run `f` against the session. `f` is an `AsyncFnOnce` so it can
    /// borrow the session across `.await` points.
    pub async fn with_session<F, R>(&self, f: F) -> Result<R>
    where
        F: for<'a> AsyncFnOnce(&'a SftpSession) -> Result<R>,
    {
        f(&self.session).await
    }
}

/// Normalise a remote path: reject `..` segments, `\`, `\0`, and empty input.
/// This is the ONLY place path normalisation happens. Every SFTP command must
/// route through `normalise_remote_path`.
pub fn normalise_remote_path(raw: &str) -> Result<String> {
    if raw.is_empty() {
        return Err(AppError::Validation("empty path".into()));
    }
    if raw.contains('\0') {
        return Err(AppError::PathTraversal);
    }
    // Reject backslash outright: POSIX SFTP servers would silently treat it as
    // a literal filename byte, but our UI and Windows-host code paths treat `\`
    // as a separator. Collapsing the ambiguity here prevents traversal payloads
    // like `/etc/\..\passwd` from slipping past the `..`-segment scan below.
    if raw.contains('\\') {
        return Err(AppError::PathTraversal);
    }
    // Disallow bare "..". Allow "." only as the current dir literal the UI
    // passes at cwd.
    for seg in raw.split('/') {
        if seg == ".." {
            return Err(AppError::PathTraversal);
        }
    }
    Ok(raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_rejects_dot_dot() {
        assert!(normalise_remote_path("/etc/../secret").is_err());
        assert!(normalise_remote_path("..").is_err());
    }

    #[test]
    fn normalise_rejects_nul() {
        assert!(normalise_remote_path("/etc/\0").is_err());
    }

    #[test]
    fn normalise_allows_dot() {
        assert!(normalise_remote_path("/home/user/.").is_ok());
    }

    #[test]
    fn normalise_rejects_empty() {
        assert!(normalise_remote_path("").is_err());
    }

    #[test]
    fn normalise_rejects_backslash() {
        assert!(normalise_remote_path("/etc/\\..\\passwd").is_err());
        assert!(normalise_remote_path("/tmp/\\foo").is_err());
    }

    #[test]
    fn normalise_allows_dotdot_prefix_segment() {
        // "..foo" is a legal filename; only bare ".." traverses.
        assert!(normalise_remote_path("/tmp/..foo").is_ok());
        assert!(normalise_remote_path("/tmp/foo..").is_ok());
    }
}
