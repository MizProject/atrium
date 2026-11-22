//! Pure-Rust systemd service protocol helpers (`sd_notify`, `sd_watchdog_enabled`,
//! and `sd_listen_fds` socket activation) without requiring external `libsystemd` FFI.

use std::env;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
#[cfg(target_os = "linux")]
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram, UnixListener};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

const SD_LISTEN_FDS_START: RawFd = 3;

const SYSTEMD_CHILD_ENV_VARS: &[&str] = &[
    "NOTIFY_SOCKET",
    "WATCHDOG_USEC",
    "WATCHDOG_PID",
    "LISTEN_FDS",
    "LISTEN_PID",
    "LISTEN_FDNAMES",
];

#[derive(Debug, Clone)]
enum NotifyTarget {
    Filesystem(PathBuf),
    Abstract(Vec<u8>),
}

impl NotifyTarget {
    fn parse(raw: &str) -> Option<Self> {
        if let Some(name) = raw.strip_prefix('@') {
            if name.is_empty() {
                return None;
            }
            Some(Self::Abstract(name.as_bytes().to_vec()))
        } else if raw.starts_with('/') {
            Some(Self::Filesystem(PathBuf::from(raw)))
        } else {
            None
        }
    }

    fn to_socket_addr(&self) -> io::Result<SocketAddr> {
        match self {
            Self::Filesystem(path) => SocketAddr::from_pathname(path),
            #[cfg(target_os = "linux")]
            Self::Abstract(bytes) => SocketAddr::from_abstract_name(bytes),
            #[cfg(not(target_os = "linux"))]
            Self::Abstract(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "abstract Unix sockets require Linux",
            )),
        }
    }
}

/// Remove systemd service notification / activation variables from spawned child commands
/// so helper or user desktop children cannot hijack `atriumd`'s readiness or socket state.
pub(crate) fn scrub_child_env(cmd: &mut Command) {
    for var in SYSTEMD_CHILD_ENV_VARS {
        cmd.env_remove(var);
    }
}

/// Client for the systemd `NOTIFY_SOCKET` notification & watchdog datagram protocol.
pub(crate) struct SystemdNotifier {
    target: Option<NotifyTarget>,
    socket: Option<UnixDatagram>,
    watchdog_interval: Option<Duration>,
    last_watchdog: Instant,
    invocation_id: Option<String>,
}

impl SystemdNotifier {
    /// Discover systemd integration state (`NOTIFY_SOCKET`, `WATCHDOG_USEC`, `INVOCATION_ID`)
    /// from the current process environment.
    pub(crate) fn from_env() -> Self {
        let notify_raw = env::var("NOTIFY_SOCKET").ok();
        let watchdog_usec = env::var("WATCHDOG_USEC").ok();
        let watchdog_pid = env::var("WATCHDOG_PID").ok();
        let invocation_id = env::var("INVOCATION_ID").ok().filter(|s| !s.is_empty());

        Self::from_parts(
            notify_raw.as_deref(),
            watchdog_usec.as_deref(),
            watchdog_pid.as_deref(),
            invocation_id,
            std::process::id(),
        )
    }

    fn from_parts(
        notify_raw: Option<&str>,
        watchdog_usec: Option<&str>,
        watchdog_pid: Option<&str>,
        invocation_id: Option<String>,
        self_pid: u32,
    ) -> Self {
        let target = notify_raw.and_then(NotifyTarget::parse);
        let socket = target
            .as_ref()
            .and_then(|_| UnixDatagram::unbound().ok());

        let watchdog_interval = if socket.is_some() {
            parse_watchdog_ping_interval(watchdog_usec, watchdog_pid, self_pid)
        } else {
            None
        };

        Self {
            target,
            socket,
            watchdog_interval,
            last_watchdog: Instant::now(),
            invocation_id,
        }
    }

    /// True if `atriumd` appears to have been launched under systemd management.
    pub(crate) fn is_systemd(&self) -> bool {
        self.socket.is_some() || self.invocation_id.is_some()
    }

    /// Configured half-interval between `WATCHDOG=1` keepalive pings, if enabled by systemd.
    pub(crate) fn watchdog_interval(&self) -> Option<Duration> {
        self.watchdog_interval
    }

    pub(crate) fn notify_ready(&self, status: &str) {
        let clean = sanitize_status(status);
        let _ = self.send_raw(&format!("READY=1\nSTATUS={clean}"));
    }

    pub(crate) fn notify_status(&self, status: &str) {
        let clean = sanitize_status(status);
        let _ = self.send_raw(&format!("STATUS={clean}"));
    }

    pub(crate) fn notify_stopping(&self, status: &str) {
        let clean = sanitize_status(status);
        let _ = self.send_raw(&format!("STOPPING=1\nSTATUS={clean}"));
    }

    pub(crate) fn notify_errno(&self, errno: i32, status: &str) {
        let clean = sanitize_status(status);
        let _ = self.send_raw(&format!("ERRNO={errno}\nSTATUS={clean}"));
    }

    /// Send `WATCHDOG=1` if watchdog pings are active and the half-interval has elapsed.
    /// Returns `true` when a keepalive datagram was transmitted.
    pub(crate) fn maybe_notify_watchdog(&mut self) -> bool {
        let Some(interval) = self.watchdog_interval else {
            return false;
        };
        if self.last_watchdog.elapsed() < interval {
            return false;
        }
        let sent = self.send_raw("WATCHDOG=1");
        self.last_watchdog = Instant::now();
        sent
    }

    fn send_raw(&self, payload: &str) -> bool {
        let (Some(sock), Some(target)) = (&self.socket, &self.target) else {
            return false;
        };
        let Ok(addr) = target.to_socket_addr() else {
            return false;
        };
        sock.send_to_addr(payload.as_bytes(), &addr).is_ok()
    }
}

/// Adopted systemd-managed Unix socket listener passed via `LISTEN_FDS` / `LISTEN_PID`.
pub(crate) struct ActivatedSocket {
    pub(crate) listener: UnixListener,
    pub(crate) bound_path: Option<PathBuf>,
}

/// Inspect `LISTEN_PID` and `LISTEN_FDS` and adopt FD 3 (`SD_LISTEN_FDS_START`) when present.
pub(crate) fn take_systemd_socket() -> Result<Option<ActivatedSocket>, String> {
    let pid_raw = match env::var("LISTEN_PID") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => return Ok(None),
    };
    let expected_pid: u32 = match pid_raw.trim().parse() {
        Ok(p) => p,
        Err(_) => return Ok(None),
    };
    if expected_pid != std::process::id() {
        return Ok(None);
    }

    let fds_raw = match env::var("LISTEN_FDS") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => return Ok(None),
    };
    let count: i32 = fds_raw
        .trim()
        .parse()
        .map_err(|e| format!("invalid LISTEN_FDS value '{fds_raw}': {e}"))?;
    if count <= 0 {
        return Ok(None);
    }

    // Close extra inherited descriptors above the single IPC listener socket to avoid FD leaks.
    for offset in 1..count {
        let extra_fd = SD_LISTEN_FDS_START + offset;
        // SAFETY: Closing extra systemd-inherited descriptors targeted exclusively to our PID.
        unsafe {
            let _ = libc::close(extra_fd);
        }
    }

    let listener = adopt_unix_listener_fd(SD_LISTEN_FDS_START)?;
    let bound_path = listener
        .local_addr()
        .ok()
        .and_then(|addr| addr.as_pathname().map(PathBuf::from));

    Ok(Some(ActivatedSocket {
        listener,
        bound_path,
    }))
}

fn adopt_unix_listener_fd(fd: RawFd) -> Result<UnixListener, String> {
    // SAFETY: `fd` is validated via `fstat` and converted into `OwnedFd` only after checks pass.
    unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        if libc::fstat(fd, &mut st) != 0 {
            return Err(format!(
                "fstat failed on systemd LISTEN_FDS fd {fd}: {}",
                io::Error::last_os_error()
            ));
        }
        if (st.st_mode & libc::S_IFMT) != libc::S_IFSOCK {
            return Err(format!("systemd fd {fd} is not a Unix socket"));
        }

        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) != 0 {
            return Err(format!(
                "failed to set FD_CLOEXEC on systemd fd {fd}: {}",
                io::Error::last_os_error()
            ));
        }

        let owned = OwnedFd::from_raw_fd(fd);
        let listener = UnixListener::from(owned);
        let _ = listener.as_raw_fd();
        Ok(listener)
    }
}

fn parse_watchdog_ping_interval(
    watchdog_usec: Option<&str>,
    watchdog_pid: Option<&str>,
    self_pid: u32,
) -> Option<Duration> {
    if let Some(pid_str) = watchdog_pid
        && !pid_str.trim().is_empty()
    {
        let parsed: u32 = pid_str.trim().parse().ok()?;
        if parsed != self_pid {
            return None;
        }
    }

    let usec: u64 = watchdog_usec?.trim().parse().ok()?;
    if usec == 0 {
        return None;
    }

    // systemd recommends sending keepalives every WatchdogSec / 2.
    Some(Duration::from_micros(usec / 2).max(Duration::from_millis(250)))
}

fn sanitize_status(status: &str) -> String {
    status
        .chars()
        .map(|c| match c {
            '\n' | '\r' => ' ',
            other => other,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_tmp_socket_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "atrium-sd-test-{}-{nanos}.sock",
            std::process::id()
        ))
    }

    #[test]
    fn notifier_transmits_ready_watchdog_stopping_and_errno() {
        let sock_path = unique_tmp_socket_path();
        let receiver = UnixDatagram::bind(&sock_path).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();

        let sock_str = sock_path.to_string_lossy().into_owned();
        let mut notifier = SystemdNotifier::from_parts(
            Some(&sock_str),
            Some("2000000"),
            Some(&std::process::id().to_string()),
            Some("inv-123".into()),
            std::process::id(),
        );
        assert!(notifier.is_systemd());
        assert_eq!(notifier.watchdog_interval(), Some(Duration::from_secs(1)));

        notifier.notify_ready("listening\nok");
        let mut buf = [0u8; 512];
        let n = receiver.recv(&mut buf).unwrap();
        assert_eq!(
            std::str::from_utf8(&buf[..n]).unwrap(),
            "READY=1\nSTATUS=listening ok"
        );

        notifier.notify_errno(libc::EINVAL, "bad config");
        let n = receiver.recv(&mut buf).unwrap();
        assert_eq!(
            std::str::from_utf8(&buf[..n]).unwrap(),
            format!("ERRNO={}\nSTATUS=bad config", libc::EINVAL)
        );

        // Force elapsed threshold cleanly for unit test without sleeping 1s.
        notifier.last_watchdog = Instant::now() - Duration::from_secs(2);
        assert!(notifier.maybe_notify_watchdog());
        let n = receiver.recv(&mut buf).unwrap();
        assert_eq!(std::str::from_utf8(&buf[..n]).unwrap(), "WATCHDOG=1");
        assert!(!notifier.maybe_notify_watchdog());

        notifier.notify_stopping("SIGTERM received");
        let n = receiver.recv(&mut buf).unwrap();
        assert_eq!(
            std::str::from_utf8(&buf[..n]).unwrap(),
            "STOPPING=1\nSTATUS=SIGTERM received"
        );

        let _ = std::fs::remove_file(&sock_path);
    }

    #[test]
    fn watchdog_ignores_mismatched_pid_and_parses_abstract_socket() {
        assert_eq!(
            parse_watchdog_ping_interval(Some("30000000"), Some("999999"), 1000),
            None
        );
        assert_eq!(
            parse_watchdog_ping_interval(Some("30000000"), Some("1000"), 1000),
            Some(Duration::from_secs(15))
        );
        assert_eq!(parse_watchdog_ping_interval(Some("0"), None, 1000), None);

        let target = NotifyTarget::parse("@systemd/notify").unwrap();
        assert!(target.to_socket_addr().is_ok());
        assert!(NotifyTarget::parse("@").is_none());
        assert!(NotifyTarget::parse("relative/notify.sock").is_none());
    }

    #[test]
    fn scrub_child_env_removes_all_systemd_keys() {
        let mut cmd = Command::new("/bin/true");
        scrub_child_env(&mut cmd);
        let removed: Vec<&OsStr> = cmd
            .get_envs()
            .filter_map(|(k, v)| if v.is_none() { Some(k) } else { None })
            .collect();
        for expected in SYSTEMD_CHILD_ENV_VARS {
            assert!(
                removed.contains(&OsStr::new(expected)),
                "missing env_remove({expected})"
            );
        }
    }
}
