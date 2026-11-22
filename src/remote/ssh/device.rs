//! This device's own login to a host: the password is typed once, and nex uses it to put a key
//! of its own in the host's `~/.ssh/authorized_keys`. Every login after that uses the key,
//! until the key expires (`device-key-days` in the config). Then the password is asked for
//! once more and the key is replaced.
//!
//! The key's line in authorized_keys carries `expiry-time="YYYYMMDD"` where the host's
//! OpenSSH understands it (8.7 and later), so the host turns the key down after that date even
//! if this device is lost. Older hosts only get the client-side expiry. The line ends with
//! this device's comment (`nex-device-1a2b3c4d`), which is how a new key replaces the old one.
//!
//! Kept in nex's config folder: `device_key` (OpenSSH private key, 0600) and `remote.json`
//! (the host, the key's comment and its expiry). The password is never saved.

use std::path::PathBuf;

use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, PrivateKey};
use serde::{Deserialize, Serialize};

use super::Server;

const DAY: u64 = 86_400;

/// The host this device logs in to, and its key there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub server: Server,
    /// The comment on this device's line in authorized_keys (`nex-device-…`).
    pub comment: String,
    /// When the key stops being used (unix seconds); `None` before a key was installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_expires: Option<u64>,
}

impl Profile {
    /// A profile for a host nex has no key on yet.
    pub fn new(server: Server) -> Profile {
        let mut id = [0u8; 4];
        rand_bytes(&mut id);
        let comment = format!("nex-device-{}", id.iter().map(|b| format!("{b:02x}")).collect::<String>());
        Profile { server, comment, key_expires: None }
    }

    /// The key can still be used at `now` (unix seconds).
    pub fn key_valid(&self, now: u64) -> bool {
        self.key_expires.is_some_and(|t| now < t)
    }
}

fn rand_bytes(out: &mut [u8]) {
    use rand::RngExt as _;
    rand::rng().fill(out);
}

fn dir() -> Option<PathBuf> {
    crate::platform::config_dir()
}

fn profile_path() -> Option<PathBuf> {
    Some(dir()?.join("remote.json"))
}

fn key_path() -> Option<PathBuf> {
    Some(dir()?.join("device_key"))
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The saved profile, if this device has connected before.
pub fn load_profile() -> Option<Profile> {
    serde_json::from_slice(&std::fs::read(profile_path()?).ok()?).ok()
}

pub fn save_profile(profile: &Profile) -> std::io::Result<()> {
    let path = profile_path().ok_or_else(|| std::io::Error::other("no config folder"))?;
    write_private(&path, &serde_json::to_vec_pretty(profile).map_err(std::io::Error::other)?)
}

/// The saved device key.
pub fn load_key() -> Option<PrivateKey> {
    PrivateKey::from_openssh(std::fs::read_to_string(key_path()?).ok()?).ok()
}

/// A new ed25519 key, saved over the old one.
pub fn new_key(comment: &str) -> std::io::Result<PrivateKey> {
    let mut key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).map_err(std::io::Error::other)?;
    key.set_comment(comment);
    let text = key.to_openssh(LineEnding::LF).map_err(std::io::Error::other)?;
    let path = key_path().ok_or_else(|| std::io::Error::other("no config folder"))?;
    write_private(&path, text.as_bytes())?;
    Ok(key)
}

/// Write `bytes` to `path` readable by this user only, through a temporary file so a crash
/// never leaves half of it.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut f = options.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

/// Whether `ssh -V` (`OpenSSH_9.6p1 …`) is a version whose sshd understands `expiry-time`.
pub fn supports_expiry(version: &str) -> bool {
    let Some(rest) = version.trim().strip_prefix("OpenSSH_") else { return false };
    let mut parts = rest.split(|c: char| !c.is_ascii_digit());
    let major: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor) >= (8, 7)
}

/// `unix` seconds as the `YYYYMMDD` date `expiry-time` takes (UTC; a day either way doesn't
/// matter here).
pub fn yyyymmdd(unix: u64) -> String {
    let days = (unix / DAY) as i64;
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}")
}

/// When a key made at `now` expires, `days` later.
pub fn expiry(now: u64, days: u32) -> u64 {
    now + u64::from(days.max(1)) * DAY
}

/// The key's line for authorized_keys: `[expiry-time="…" ]ssh-ed25519 AAAA… comment`.
pub fn authorized_line(key: &PrivateKey, comment: &str, expires: Option<u64>) -> Result<String, String> {
    let public = key.public_key().to_openssh().map_err(|e| e.to_string())?;
    // `to_openssh` ends with the key's own comment; ours goes there instead.
    let mut words = public.split_whitespace();
    let (algo, data) = (words.next().unwrap_or_default(), words.next().unwrap_or_default());
    let options = expires.map(|t| format!("expiry-time=\"{}\" ", yyyymmdd(t))).unwrap_or_default();
    Ok(format!("{options}{algo} {data} {comment}"))
}

/// The host's OpenSSH version, for [`supports_expiry`].
pub const VERSION_COMMAND: &str = "ssh -V 2>&1; true";

/// A script for the host that puts `line` in `~/.ssh/authorized_keys`, dropping any line that
/// ends with `comment` (this device's previous key). The file is rewritten in place (`cat >`),
/// so it keeps its owner, mode and SELinux label.
pub fn install_script(line: &str, comment: &str) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
    format!(
        "umask 077 && mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys && \
         t=$(mktemp ~/.ssh/.nex-keys.XXXXXX) && \
         {{ grep -v -- {pattern} ~/.ssh/authorized_keys || true; }} > \"$t\" && \
         printf '%s\\n' {line} >> \"$t\" && cat \"$t\" > ~/.ssh/authorized_keys && rm -f \"$t\"",
        pattern = q(&format!(" {comment}$")),
        line = q(line),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_for_expiry_time() {
        assert_eq!(yyyymmdd(0), "19700101");
        assert_eq!(yyyymmdd(1_700_000_000), "20231114");
        assert_eq!(yyyymmdd(951_782_400), "20000229");
    }

    #[test]
    fn expiry_needs_openssh_8_7() {
        assert!(supports_expiry("OpenSSH_9.6p1 Ubuntu-3ubuntu13, OpenSSL 3.0.13 30 Jan 2024"));
        assert!(supports_expiry("OpenSSH_8.7p1"));
        assert!(!supports_expiry("OpenSSH_8.4p1 Debian-5"));
        assert!(!supports_expiry("Dropbear v2022.83"));
        assert!(!supports_expiry(""));
    }

    #[test]
    fn keys_expire_after_their_days() {
        let p = Profile { server: Server { host: "h".into(), port: 22, user: "u".into() }, comment: "c".into(), key_expires: Some(expiry(1000, 30)) };
        assert!(p.key_valid(1000));
        assert!(p.key_valid(1000 + 29 * DAY));
        assert!(!p.key_valid(1000 + 30 * DAY));
        assert!(!Profile { key_expires: None, ..p }.key_valid(0));
    }

    #[test]
    fn authorized_lines_carry_the_expiry_and_our_comment() {
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let line = authorized_line(&key, "nex-device-00ff00ff", Some(0)).unwrap();
        assert!(line.starts_with("expiry-time=\"19700101\" ssh-ed25519 AAAA"), "{line}");
        assert!(line.ends_with(" nex-device-00ff00ff"));
        let line = authorized_line(&key, "nex-device-00ff00ff", None).unwrap();
        assert!(line.starts_with("ssh-ed25519 "));
    }

    #[test]
    fn new_profiles_get_their_own_comment() {
        let s = Server { host: "h".into(), port: 22, user: "u".into() };
        let (a, b) = (Profile::new(s.clone()), Profile::new(s));
        assert!(a.comment.starts_with("nex-device-") && a.comment.len() == "nex-device-".len() + 8);
        assert_ne!(a.comment, b.comment);
    }

    /// The install script, run against a scratch home: it adds our line, replaces our old
    /// line, and keeps everyone else's.
    #[cfg(unix)]
    #[test]
    fn install_script_replaces_only_our_line() {
        let home = std::env::temp_dir().join(format!("nex-keys-test-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        let keys = home.join(".ssh/authorized_keys");
        std::fs::write(&keys, "ssh-ed25519 AAAAother someone@laptop\nssh-ed25519 AAAAold nex-device-1\n").unwrap();
        let run = |line: &str| {
            let ok = std::process::Command::new("sh").arg("-c").arg(install_script(line, "nex-device-1")).env("HOME", &home).status().unwrap();
            assert!(ok.success());
        };
        run("expiry-time=\"20270101\" ssh-ed25519 AAAAnew nex-device-1");
        let text = std::fs::read_to_string(&keys).unwrap();
        assert_eq!(text, "ssh-ed25519 AAAAother someone@laptop\nexpiry-time=\"20270101\" ssh-ed25519 AAAAnew nex-device-1\n");
        // A device with a similar comment is someone else's line.
        std::fs::write(&keys, "ssh-ed25519 AAAAx nex-device-10\n").unwrap();
        run("ssh-ed25519 AAAAy nex-device-1");
        assert_eq!(std::fs::read_to_string(&keys).unwrap(), "ssh-ed25519 AAAAx nex-device-10\nssh-ed25519 AAAAy nex-device-1\n");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
