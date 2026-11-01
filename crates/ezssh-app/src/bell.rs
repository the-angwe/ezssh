//! `terminal.bell = sound`: the webview's 880 Hz, 80 ms tick.
//!
//! No audio library: Windows has `Beep`; elsewhere a tiny WAV of the same
//! tone is written once to the temp directory and handed to the first
//! player found (`pw-play`, `paplay`, `aplay` on Linux, `afplay` on macOS).
//! Playback runs on a short-lived thread and never blocks the UI. A
//! failure is logged once; the bell is a nicety, not worth a toast.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const FREQ_HZ: u32 = 880;
const MILLIS: u32 = 80;

static WARNED: AtomicBool = AtomicBool::new(false);
static LAST_MS: AtomicU64 = AtomicU64::new(0);

/// Ring, at most once per 100 ms (a program printing BEL in a loop must
/// not queue up seconds of beeps).
pub fn ring() {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let last = LAST_MS.swap(now, Ordering::Relaxed);
    if now.saturating_sub(last) < 100 {
        return;
    }
    let spawned = std::thread::Builder::new().name("bell".into()).spawn(|| {
        if let Err(e) = play() {
            if !WARNED.swap(true, Ordering::Relaxed) {
                tracing::warn!("bell: cannot play the bell sound: {e}");
            } else {
                tracing::debug!("bell: {e}");
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("bell: thread spawn failed: {e}");
    }
}

#[cfg(windows)]
fn play() -> Result<(), String> {
    // SAFETY: Beep takes two integers and has no memory preconditions.
    let ok = unsafe { windows_sys::Win32::System::Diagnostics::Debug::Beep(FREQ_HZ, MILLIS) };
    if ok == 0 { Err(format!("Beep failed: {}", std::io::Error::last_os_error())) } else { Ok(()) }
}

#[cfg(not(windows))]
fn play() -> Result<(), String> {
    let path = std::env::temp_dir().join(format!("ezssh-bell-{FREQ_HZ}hz-{MILLIS}ms.wav"));
    if !path.exists() {
        std::fs::write(&path, wav()).map_err(|e| format!("writing {}: {e}", path.display()))?;
    }
    let players: &[&str] = if cfg!(target_os = "macos") { &["afplay"] } else { &["pw-play", "paplay", "aplay"] };
    let mut tried = Vec::new();
    for player in players {
        let mut cmd = std::process::Command::new(player);
        if *player == "aplay" {
            cmd.arg("-q");
        }
        match cmd.arg(&path).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status() {
            Ok(s) if s.success() => return Ok(()),
            Ok(s) => tried.push(format!("{player} exited with {s}")),
            Err(e) => tried.push(format!("{player}: {e}")),
        }
    }
    Err(tried.join("; "))
}

/// 16-bit mono PCM WAV of the tone, with 5 ms fades so it does not click.
#[cfg(any(not(windows), test))]
fn wav() -> Vec<u8> {
    const RATE: u32 = 22_050;
    let n = RATE * MILLIS / 1000;
    let fade = RATE * 5 / 1000;
    let mut data = Vec::with_capacity(n as usize * 2);
    for i in 0..n {
        let env = (i.min(n - 1 - i) as f32 / fade as f32).min(1.0);
        let s = (2.0 * std::f32::consts::PI * FREQ_HZ as f32 * i as f32 / RATE as f32).sin() * 0.25 * env;
        data.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn wav_header_is_consistent() {
        let w = super::wav();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        let data_len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(w.len(), 44 + data_len);
        assert_eq!(data_len, 22_050 * 80 / 1000 * 2);
    }
}
