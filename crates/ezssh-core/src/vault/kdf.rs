use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KdfParams {
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            m_cost_kib: 64 * 1024,
            t_cost: 3,
            p_cost: 1,
        }
    }
}

/// Upper bounds for KDF parameters read from untrusted input (a backup
/// envelope, a database row). argon2 accepts `m_cost` up to `u32::MAX`
/// KiB and allocates it up front, so a crafted file could otherwise
/// abort the process or pin a worker for minutes.
pub const MAX_KDF_M_COST_KIB: u32 = 1024 * 1024;
pub const MAX_KDF_T_COST: u32 = 16;
pub const MAX_KDF_P_COST: u32 = 8;

/// `true` when the parameters are within the window this app is willing
/// to run (at least argon2's own minimums, at most the caps above).
pub fn params_within_limits(p: &KdfParams) -> bool {
    (8 * 1024..=MAX_KDF_M_COST_KIB).contains(&p.m_cost_kib)
        && (1..=MAX_KDF_T_COST).contains(&p.t_cost)
        && (1..=MAX_KDF_P_COST).contains(&p.p_cost)
}

/// `derive_key` on tokio's blocking pool. Argon2id with the default
/// parameters takes 100–300 ms and 64 MiB; the connection runtime has
/// two workers, so running it inline would stall every SSH reader and
/// transfer for that long.
pub async fn derive_key_blocking(
    password: Zeroizing<Vec<u8>>,
    salt: Vec<u8>,
    params: KdfParams,
) -> crate::error::Result<Zeroizing<[u8; 32]>> {
    tokio::task::spawn_blocking(move || derive_key(&password, &salt, params))
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "kdf: blocking task failed");
            crate::error::AppError::Crypto
        })?
}

pub fn derive_key(
    password: &[u8],
    salt: &[u8],
    params: KdfParams,
) -> crate::error::Result<Zeroizing<[u8; 32]>> {
    let mut out = Zeroizing::new([0u8; 32]);
    let p = Params::new(params.m_cost_kib, params.t_cost, params.p_cost, Some(32))
        .map_err(|_| crate::error::AppError::Crypto)?;
    let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    a2.hash_password_into(password, salt, &mut *out)
        .map_err(|_| crate::error::AppError::Crypto)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_is_deterministic_for_same_inputs() {
        let salt = [7u8; 16];
        let p = KdfParams {
            m_cost_kib: 8 * 1024,
            t_cost: 1,
            p_cost: 1,
        }; // fast for tests
        let a = derive_key(b"correct horse", &salt, p).unwrap();
        let b = derive_key(b"correct horse", &salt, p).unwrap();
        assert_eq!(*a, *b);
    }

    #[test]
    fn limits_window() {
        assert!(params_within_limits(&KdfParams::default()));
        assert!(params_within_limits(&KdfParams {
            m_cost_kib: 8 * 1024,
            t_cost: 1,
            p_cost: 1
        }));
        assert!(!params_within_limits(&KdfParams {
            m_cost_kib: 4_000_000_000,
            t_cost: 3,
            p_cost: 1
        }));
        assert!(!params_within_limits(&KdfParams {
            m_cost_kib: 64 * 1024,
            t_cost: 0,
            p_cost: 1
        }));
        assert!(!params_within_limits(&KdfParams {
            m_cost_kib: 64 * 1024,
            t_cost: 3,
            p_cost: 64
        }));
    }

    #[tokio::test]
    async fn blocking_wrapper_matches_inline() {
        let salt = [7u8; 16];
        let p = KdfParams {
            m_cost_kib: 8 * 1024,
            t_cost: 1,
            p_cost: 1,
        };
        let a = derive_key(b"correct horse", &salt, p).unwrap();
        let b = derive_key_blocking(Zeroizing::new(b"correct horse".to_vec()), salt.to_vec(), p)
            .await
            .unwrap();
        assert_eq!(*a, *b);
    }

    #[test]
    fn derive_differs_with_different_password() {
        let salt = [7u8; 16];
        let p = KdfParams {
            m_cost_kib: 8 * 1024,
            t_cost: 1,
            p_cost: 1,
        };
        let a = derive_key(b"aaa", &salt, p).unwrap();
        let b = derive_key(b"bbb", &salt, p).unwrap();
        assert_ne!(*a, *b);
    }
}
