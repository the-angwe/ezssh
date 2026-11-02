# Security policy

ezSsh stores SSH credentials and private keys, encrypts them at rest with a
master password, and establishes network connections to servers the user
trusts. Security issues are handled as the highest-priority class of bug.

## Supported versions

Only the current `main` branch and the most recent tagged release receive
security fixes. Older tags are not backported. Upgrading is always the
recommended remediation.

| Version | Supported |
|---|---|
| `main` (unreleased) | ✅ |
| Latest release (`v2.0.x`, including pre-releases) | ✅ |
| Older tags | ❌ |

## Reporting a vulnerability

**Do not open public GitHub issues for security reports.**

Preferred channel: [GitHub Private Security
Advisory](https://github.com/the-angwe/ezssh/security/advisories/new).
Private advisories keep the discussion off the public tracker and let us
coordinate a fix + disclosure without exposing users.

Alternative channel: email the maintainer directly. Contact details are in
the repository's `CODEOWNERS` / commit history. For encrypted mail please
request a PGP key in your first message.

When reporting, please include:
- A description of the issue and the ezSsh version / commit it reproduces
  against.
- Steps to reproduce, or a proof-of-concept if you have one.
- Your assessment of the impact (information disclosure, auth bypass,
  code execution, etc.) and the minimum privileges required to trigger it.
- Whether you plan to disclose publicly, and your preferred timeline.

## Response

- Acknowledgement of the report: within **5 business days**.
- Triage and initial assessment (severity, scope): within **10 business days**.
- Fix + release: as fast as the fix allows. Critical issues are targeted
  for a patch release within **30 days**; lower severity follow the normal
  release cadence.

We will keep you informed at each step. If the issue affects a third-party
dependency, we will coordinate with upstream and credit you in the fix
announcement.

## Disclosure policy

We follow **coordinated disclosure**. The advisory is published (and the
fix shipped) after:
1. A fix is merged and released, OR
2. Ninety days have passed since the initial report, whichever is sooner.

Reporters who want public credit will be named in the release notes and
the GitHub advisory. Anonymous reports are welcome — tell us if you want
to stay anonymous.

## Scope

In scope:
- Any defect that allows an attacker to read, forge, or bypass:
  - The encrypted credential vault (master password, Argon2 KDF,
    ChaCha20-Poly1305 AEAD).
  - Known-hosts verification (host-key TOFU, mismatch handling).
  - The SSH / SFTP protocol wrappers around `russh`.
- Defects that leak secrets via logs, events, or error messages.
- Input validation bypasses in the core API (for example path handling in
  SFTP / WSL file operations or restored backups).
- Cryptographic weaknesses in how we use `argon2`, `chacha20poly1305`,
  `russh`, or `russh-keys`.

Out of scope:
- Vulnerabilities in third-party SSH servers, X11 servers (VcXsrv), WSL
  distros, or remote shells. Report those upstream.
- Physical attacks on an unlocked device.
- Attacks that require the attacker to already have write access to the
  user's ezSsh data directory (shared with ezTerm).
- Missing security headers on repository landing pages, typosquatted
  lookalike domains, and similar non-ezSsh issues.

## Hardening currently in place

The points below describe what ezSsh does today; they are not a warranty.
If you find any of them broken, that *is* a security issue — please report.

- **Master password → key derivation** — Argon2id with memory-hard
  parameters (see `crates/ezssh-core/src/vault/kdf.rs`). Salt is per-vault,
  randomised at init time.
- **Secret encryption** — ChaCha20-Poly1305 AEAD with a fresh 96-bit
  nonce per ciphertext. Verifier blob proves the password is correct
  without revealing it.
- **In-memory hygiene** — secrets flow through `zeroize::Zeroizing`
  wrappers so plaintexts are scrubbed when dropped. The remaining
  exposure is documented on `crates/ezssh-core/src/ssh/client.rs::AuthMaterial`
  (russh keeps one internal copy that it doesn't zeroise; tracked).
- **Known-hosts** — TOFU on first connect, hard-fail on subsequent
  mismatches unless the user explicitly chooses to replace.
- **Core API surface** — every `ezssh_core::api` function that touches
  vault-protected data calls `require_unlocked(&state)` before any work;
  the only exceptions are the `vault_*` functions, the updater and a narrow
  `settings_get` allow-list for pre-unlock theme loading.
- **Events never carry plaintext secrets.** The UI refers to credentials
  by ID; decryption happens inside `ezssh-core` at connect time.
- **Signed updates** — the updater installs only files whose minisign
  signature verifies against the public key compiled into the app
  (`crates/ezssh-core/updater.pub`) and whose signed trusted comment names
  the announced version; the signature is checked while downloading and a
  file that fails is deleted. No web content is rendered, so there is no
  script surface.

## Dependency policy

We pin to stable, actively maintained Rust crates. `cargo audit` runs in
CI on every pull request and push to `main`. Known-vulnerable dependencies block a release.

No paid dependencies. Adding a new dependency requires maintainer
review.

## Acknowledgements

Researchers whose reports have led to fixes will be credited here.
