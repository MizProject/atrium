//! Systemd service management helpers for initializing, inspecting, installing,
//! and cleanly stopping `atriumd` via `systemctl` and `journalctl`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode, Stdio};
use std::time::{Duration, Instant};

use clap::{Args, Subcommand};

use atrium_core::ipc::{self, DaemonClient};

#[derive(Args, Clone, Debug, Default)]
pub(crate) struct SystemdTargetArgs {
    /// Manage user-session systemd service (`systemctl --user`) instead of system template
    #[arg(long)]
    pub(crate) user: bool,

    /// Explicit systemd unit name override (e.g. `atriumd@miku.service` or `atriumd.service`)
    #[arg(long)]
    pub(crate) unit: Option<String>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum SystemdAction {
    /// Install the atriumd systemd unit file and reload systemd manager configuration
    Install {
        #[command(flatten)]
        target: SystemdTargetArgs,

        /// Explicit path to the `atriumd` binary used in ExecStart (defaults to sibling `atriumd`)
        #[arg(long)]
        bin: Option<PathBuf>,

        /// Enable the service to start automatically on boot/login
        #[arg(long)]
        enable: bool,

        /// Start the service immediately after installation
        #[arg(long)]
        start: bool,
    },
    /// Stop, disable, remove the installed atriumd unit file, and reload systemd
    Uninstall {
        #[command(flatten)]
        target: SystemdTargetArgs,
    },
    /// Start atriumd via systemd and verify IPC socket readiness
    Start {
        #[command(flatten)]
        target: SystemdTargetArgs,
    },
    /// Stop atriumd cleanly via systemd and verify teardown
    Stop {
        #[command(flatten)]
        target: SystemdTargetArgs,
    },
    /// Restart atriumd cleanly via systemd
    Restart {
        #[command(flatten)]
        target: SystemdTargetArgs,
    },
    /// Enable atriumd systemd service at boot/login
    Enable {
        #[command(flatten)]
        target: SystemdTargetArgs,
    },
    /// Disable atriumd systemd service at boot/login
    Disable {
        #[command(flatten)]
        target: SystemdTargetArgs,
    },
    /// Display systemd unit state alongside IPC daemon status
    Status {
        #[command(flatten)]
        target: SystemdTargetArgs,
    },
    /// Print recent atriumd logs from journald
    Logs {
        #[command(flatten)]
        target: SystemdTargetArgs,

        /// Number of recent journal lines to show
        #[arg(long, default_value_t = 50)]
        lines: usize,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnitScope {
    System,
    User,
}

impl UnitScope {
    fn from_user_flag(user: bool) -> Self {
        if user { Self::User } else { Self::System }
    }

    fn label(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
        }
    }
}

pub(crate) fn is_valid_unit_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && name.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '-' | '_' | ':')
        })
}

pub(crate) fn resolve_unit_name(
    scope: UnitScope,
    explicit: Option<&str>,
) -> Result<String, String> {
    if let Some(raw) = explicit {
        let trimmed = raw.trim();
        if !is_valid_unit_name(trimmed) {
            return Err(format!("invalid systemd unit name '{trimmed}'"));
        }
        return Ok(trimmed.to_string());
    }

    match scope {
        UnitScope::User => Ok("atriumd.service".to_string()),
        UnitScope::System => {
            if let Some(uid) = effective_human_uid()
                && let Some(acct) = ipc::user_account_for_uid(uid)
            {
                return Ok(format!("atriumd@{}.service", acct.username));
            }
            Err(
                "could not resolve local non-root username for template unit; pass --unit atriumd@<user>.service"
                    .into(),
            )
        }
    }
}

fn effective_human_uid() -> Option<u32> {
    let uid = ipc::current_uid();
    if uid != 0 {
        return Some(uid);
    }
    std::env::var("SUDO_UID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|&u| u != 0)
}

pub(crate) fn render_system_template_unit(bin_path: &Path) -> Result<String, String> {
    let exe = format_exec_path(bin_path)?;
    Ok(format!(
        "[Unit]\n\
         Description=Atrium Hardware Driver Daemon for %i\n\
         Documentation=https://github.com/MizProject/atrium\n\
         After=local-fs.target systemd-udev-settle.service\n\
         Wants=systemd-udev-settle.service\n\
         \n\
         [Service]\n\
         Type=notify\n\
         NotifyAccess=main\n\
         ExecStart={exe} --allow-user %i --socket {}/{}\n\
         RuntimeDirectory=atrium\n\
         RuntimeDirectoryMode=0700\n\
         StateDirectory=atrium\n\
         StateDirectoryMode=0700\n\
         Restart=on-failure\n\
         RestartSec=2s\n\
         WatchdogSec=30s\n\
         TimeoutStopSec=10s\n\
         KillMode=mixed\n\
         StandardOutput=journal\n\
         StandardError=journal\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        ipc::SYSTEM_SOCKET_DIR,
        ipc::SOCKET_FILENAME,
    ))
}

pub(crate) fn render_user_service_unit(bin_path: &Path) -> Result<String, String> {
    let exe = format_exec_path(bin_path)?;
    Ok(format!(
        "[Unit]\n\
         Description=Atrium Hardware Driver Daemon (User Session)\n\
         Documentation=https://github.com/MizProject/atrium\n\
         After=graphical-session.target\n\
         \n\
         [Service]\n\
         Type=notify\n\
         NotifyAccess=main\n\
         ExecStart={exe} --socket %t/atrium/{}\n\
         Restart=on-failure\n\
         RestartSec=2s\n\
         WatchdogSec=30s\n\
         TimeoutStopSec=10s\n\
         KillMode=mixed\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        ipc::SOCKET_FILENAME,
    ))
}

fn format_exec_path(bin_path: &Path) -> Result<String, String> {
    let raw = bin_path.to_string_lossy();
    if raw.contains('\n') || raw.contains('\r') {
        return Err("atriumd binary path contains unsupported newline characters".into());
    }
    if raw.contains(char::is_whitespace) || raw.contains('"') {
        let escaped = raw.replace('\\', "\\\\").replace('"', "\\\"");
        Ok(format!("\"{escaped}\""))
    } else {
        Ok(raw.into_owned())
    }
}

pub(crate) fn user_systemd_dir() -> Result<PathBuf, String> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Ok(PathBuf::from(xdg).join("systemd").join("user"));
    }
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            ipc::user_account_for_uid(ipc::current_uid()).map(|u| u.home_dir)
        })
        .ok_or_else(|| "cannot resolve user config directory for systemd user unit".to_string())?;
    Ok(home.join(".config").join("systemd").join("user"))
}

fn install_unit_file_path(scope: UnitScope, explicit_unit: Option<&str>) -> Result<PathBuf, String> {
    match scope {
        UnitScope::System => {
            let file_name = match explicit_unit {
                Some(u) => {
                    if !is_valid_unit_name(u) {
                        return Err(format!("invalid unit name '{u}'"));
                    }
                    if u.ends_with(".service") {
                        u.to_string()
                    } else {
                        format!("{u}.service")
                    }
                }
                None => "atriumd@.service".to_string(),
            };
            Ok(PathBuf::from("/etc/systemd/system").join(file_name))
        }
        UnitScope::User => {
            let dir = user_systemd_dir()?;
            let file_name = match explicit_unit {
                Some(u) => {
                    if !is_valid_unit_name(u) {
                        return Err(format!("invalid unit name '{u}'"));
                    }
                    u.to_string()
                }
                None => "atriumd.service".to_string(),
            };
            Ok(dir.join(file_name))
        }
    }
}

pub(crate) fn handle_systemd_action(
    action: SystemdAction,
    sibling_bin: Option<PathBuf>,
    socket: Option<&Path>,
    no_sudo: bool,
) -> Result<(), (String, Option<ExitCode>)> {
    match action {
        SystemdAction::Install {
            target,
            bin,
            enable,
            start,
        } => {
            let scope = UnitScope::from_user_flag(target.user);
            let unit = resolve_unit_name(scope, target.unit.as_deref())
                .map_err(|e| (e, None))?;
            let resolved_bin = resolve_install_binary(bin, sibling_bin).map_err(|e| (e, None))?;
            let content = match scope {
                UnitScope::System => render_system_template_unit(&resolved_bin),
                UnitScope::User => render_user_service_unit(&resolved_bin),
            }
            .map_err(|e| (e, None))?;
            let dest = install_unit_file_path(scope, target.unit.as_deref())
                .map_err(|e| (e, None))?;

            write_unit_file(scope, &dest, &content, no_sudo).map_err(|e| (e, None))?;
            run_systemctl(scope, &["daemon-reload"], no_sudo)?;
            println!(
                "Installed {} systemd unit: {} (ExecStart={})",
                scope.label(),
                dest.display(),
                resolved_bin.display()
            );

            if enable {
                run_systemctl(scope, &["enable", &unit], no_sudo)?;
                println!("Enabled {unit}.");
            }
            if start {
                start_systemd_unit(&target, socket, no_sudo)?;
            } else if !enable {
                let flag = if scope == UnitScope::User { "--user " } else { "" };
                println!(
                    "Next steps:\n  systemctl {flag}enable --now {unit}\n  atrium daemon status --systemd{}",
                    if scope == UnitScope::User { " --user" } else { "" }
                );
            }
            Ok(())
        }
        SystemdAction::Uninstall { target } => {
            let scope = UnitScope::from_user_flag(target.user);
            let unit = resolve_unit_name(scope, target.unit.as_deref())
                .map_err(|e| (e, None))?;
            let dest = install_unit_file_path(scope, target.unit.as_deref())
                .map_err(|e| (e, None))?;

            // Best-effort stop & disable before deleting unit file so systemd state stays clean.
            let _ = run_systemctl_quiet(scope, &["disable", "--now", &unit], no_sudo);
            remove_unit_file(scope, &dest, no_sudo).map_err(|e| (e, None))?;
            run_systemctl(scope, &["daemon-reload"], no_sudo)?;
            println!("Uninstalled {} systemd unit: {}", scope.label(), dest.display());
            Ok(())
        }
        SystemdAction::Start { target } => start_systemd_unit(&target, socket, no_sudo),
        SystemdAction::Stop { target } => stop_systemd_unit(&target, socket, no_sudo),
        SystemdAction::Restart { target } => restart_systemd_unit(&target, socket, no_sudo),
        SystemdAction::Enable { target } => {
            let scope = UnitScope::from_user_flag(target.user);
            let unit = resolve_unit_name(scope, target.unit.as_deref())
                .map_err(|e| (e, None))?;
            run_systemctl(scope, &["enable", &unit], no_sudo)?;
            println!("Enabled {} unit {unit}.", scope.label());
            Ok(())
        }
        SystemdAction::Disable { target } => {
            let scope = UnitScope::from_user_flag(target.user);
            let unit = resolve_unit_name(scope, target.unit.as_deref())
                .map_err(|e| (e, None))?;
            run_systemctl(scope, &["disable", &unit], no_sudo)?;
            println!("Disabled {} unit {unit}.", scope.label());
            Ok(())
        }
        SystemdAction::Status { target } => print_systemd_status(&target, socket),
        SystemdAction::Logs { target, lines } => {
            let scope = UnitScope::from_user_flag(target.user);
            let unit = resolve_unit_name(scope, target.unit.as_deref())
                .map_err(|e| (e, None))?;
            let n_str = lines.max(1).to_string();
            let mut cmd = ProcessCommand::new("journalctl");
            if scope == UnitScope::User {
                cmd.arg("--user");
            }
            cmd.args(["-u", &unit, "-n", &n_str, "--no-pager"]);
            let status = cmd
                .status()
                .map_err(|e| (format!("failed to invoke journalctl: {e}"), None))?;
            if status.success() {
                Ok(())
            } else {
                Err((
                    format!("journalctl exited with status {status}"),
                    status.code().map(|c| ExitCode::from(c as u8)),
                ))
            }
        }
    }
}

pub(crate) fn start_systemd_unit(
    target: &SystemdTargetArgs,
    socket: Option<&Path>,
    no_sudo: bool,
) -> Result<(), (String, Option<ExitCode>)> {
    let scope = UnitScope::from_user_flag(target.user);
    let unit = resolve_unit_name(scope, target.unit.as_deref()).map_err(|e| (e, None))?;
    ensure_unit_present(scope, &unit).map_err(|e| (e, None))?;
    run_systemctl(scope, &["start", &unit], no_sudo)?;

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(mut client) = DaemonClient::connect(socket)
            && let Ok(st) = client.ping()
        {
            println!(
                "Started {unit} cleanly (pid={}, socket={}).",
                st.daemon_pid, st.socket_path
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    println!("Requested systemd start for {unit}. Run `atrium daemon status --systemd` to inspect readiness.");
    Ok(())
}

pub(crate) fn stop_systemd_unit(
    target: &SystemdTargetArgs,
    socket: Option<&Path>,
    no_sudo: bool,
) -> Result<(), (String, Option<ExitCode>)> {
    let scope = UnitScope::from_user_flag(target.user);
    let unit = resolve_unit_name(scope, target.unit.as_deref()).map_err(|e| (e, None))?;
    run_systemctl(scope, &["stop", &unit], no_sudo)?;

    let sock = ipc::discover_client_socket_path(socket);
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !sock.exists() && DaemonClient::connect(socket).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    println!("Stopped {unit} via systemd.");
    Ok(())
}

pub(crate) fn restart_systemd_unit(
    target: &SystemdTargetArgs,
    socket: Option<&Path>,
    no_sudo: bool,
) -> Result<(), (String, Option<ExitCode>)> {
    let scope = UnitScope::from_user_flag(target.user);
    let unit = resolve_unit_name(scope, target.unit.as_deref()).map_err(|e| (e, None))?;
    ensure_unit_present(scope, &unit).map_err(|e| (e, None))?;
    run_systemctl(scope, &["restart", &unit], no_sudo)?;

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(mut client) = DaemonClient::connect(socket)
            && let Ok(st) = client.ping()
        {
            println!(
                "Restarted {unit} cleanly (pid={}, socket={}).",
                st.daemon_pid, st.socket_path
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("Restarted {unit} via systemd.");
    Ok(())
}

pub(crate) fn print_systemd_status(
    target: &SystemdTargetArgs,
    socket: Option<&Path>,
) -> Result<(), (String, Option<ExitCode>)> {
    let scope = UnitScope::from_user_flag(target.user);
    let unit = resolve_unit_name(scope, target.unit.as_deref()).map_err(|e| (e, None))?;

    let mut cmd = ProcessCommand::new("systemctl");
    if scope == UnitScope::User {
        cmd.arg("--user");
    }
    cmd.args(["status", "--no-pager", &unit]);
    let status = cmd
        .status()
        .map_err(|e| (format!("failed to execute systemctl: {e}"), None))?;

    println!();
    match DaemonClient::connect(socket) {
        Ok(mut client) => match client.ping() {
            Ok(st) => {
                println!(
                    "IPC Probe       : reachable (pid={}, socket={}, systemd={})",
                    st.daemon_pid, st.socket_path, st.systemd_managed
                );
            }
            Err(e) => println!("IPC Probe       : ping error: {e}"),
        },
        Err(e) => println!(
            "IPC Probe       : unreachable at {} ({e})",
            ipc::discover_client_socket_path(socket).display()
        ),
    }

    if status.success() {
        Ok(())
    } else {
        // systemctl status returns exit 3 for inactive units; pass through cleanly.
        Err((
            format!("systemctl status returned {status} for {unit}"),
            status.code().map(|c| ExitCode::from(c as u8)),
        ))
    }
}

pub(crate) fn query_unit_active_state(scope: UnitScope, unit: &str) -> Option<String> {
    let mut cmd = ProcessCommand::new("systemctl");
    if scope == UnitScope::User {
        cmd.arg("--user");
    }
    cmd.args(["is-active", unit]);
    let out = cmd.output().ok()?;
    let txt = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if txt.is_empty() { None } else { Some(txt) }
}

fn ensure_unit_present(scope: UnitScope, unit: &str) -> Result<(), String> {
    let mut cmd = ProcessCommand::new("systemctl");
    if scope == UnitScope::User {
        cmd.arg("--user");
    }
    cmd.args(["cat", unit])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let ok = cmd.status().is_ok_and(|s| s.success());
    if ok {
        Ok(())
    } else {
        let user_flag = if scope == UnitScope::User { " --user" } else { "" };
        Err(format!(
            "systemd unit '{unit}' is not installed. Install it first with:\n  atrium daemon systemd install{user_flag}"
        ))
    }
}

fn resolve_install_binary(
    explicit: Option<PathBuf>,
    sibling_bin: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let raw = explicit
        .or(sibling_bin)
        .unwrap_or_else(|| PathBuf::from("/usr/local/bin/atriumd"));
    let canonical = if raw.exists() {
        fs::canonicalize(&raw).unwrap_or(raw)
    } else {
        return Err(format!(
            "atriumd binary not found at {}; build the workspace or pass --bin <PATH>",
            raw.display()
        ));
    };
    Ok(canonical)
}

fn write_unit_file(
    scope: UnitScope,
    dest: &Path,
    content: &str,
    no_sudo: bool,
) -> Result<(), String> {
    let need_sudo = scope == UnitScope::System && ipc::current_uid() != 0;
    if !need_sudo {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
        }
        fs::write(dest, content)
            .map_err(|e| format!("failed to write {}: {e}", dest.display()))?;
        return Ok(());
    }

    if no_sudo {
        return Err(format!(
            "writing {} requires root privileges and --no-sudo was specified",
            dest.display()
        ));
    }

    let parent = dest
        .parent()
        .ok_or_else(|| "unit destination has no parent directory".to_string())?;
    let mkdir_status = ProcessCommand::new("sudo")
        .args(["install", "-d", "-m", "0755"])
        .arg(parent)
        .status()
        .map_err(|e| format!("failed to invoke sudo install: {e}"))?;
    if !mkdir_status.success() {
        return Err(format!(
            "sudo install -d failed for {}",
            parent.display()
        ));
    }

    let mut child = ProcessCommand::new("sudo")
        .arg("tee")
        .arg(dest)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to invoke sudo tee: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(content.as_bytes())
            .map_err(|e| format!("failed writing unit file via sudo tee: {e}"))?;
    }
    let status = child
        .wait()
        .map_err(|e| format!("sudo tee failed waiting for exit: {e}"))?;
    if !status.success() {
        return Err(format!("sudo tee failed writing {}", dest.display()));
    }
    Ok(())
}

fn remove_unit_file(scope: UnitScope, dest: &Path, no_sudo: bool) -> Result<(), String> {
    if !dest.exists() {
        return Ok(());
    }
    let need_sudo = scope == UnitScope::System && ipc::current_uid() != 0;
    if !need_sudo {
        fs::remove_file(dest)
            .map_err(|e| format!("failed to remove {}: {e}", dest.display()))?;
        return Ok(());
    }
    if no_sudo {
        return Err(format!(
            "removing {} requires root privileges and --no-sudo was specified",
            dest.display()
        ));
    }
    let status = ProcessCommand::new("sudo")
        .args(["rm", "-f"])
        .arg(dest)
        .status()
        .map_err(|e| format!("failed to run sudo rm: {e}"))?;
    if !status.success() {
        return Err(format!("sudo rm failed for {}", dest.display()));
    }
    Ok(())
}

fn build_systemctl_cmd(scope: UnitScope, args: &[&str], no_sudo: bool) -> Result<ProcessCommand, String> {
    let mut cmd = if scope == UnitScope::System && ipc::current_uid() != 0 {
        if no_sudo {
            return Err(format!(
                "systemctl {} requires root privileges and --no-sudo was specified",
                args.join(" ")
            ));
        }
        let mut c = ProcessCommand::new("sudo");
        c.arg("systemctl");
        c
    } else {
        ProcessCommand::new("systemctl")
    };
    if scope == UnitScope::User {
        cmd.arg("--user");
    }
    cmd.args(args);
    Ok(cmd)
}

fn run_systemctl(
    scope: UnitScope,
    args: &[&str],
    no_sudo: bool,
) -> Result<(), (String, Option<ExitCode>)> {
    let mut cmd = build_systemctl_cmd(scope, args, no_sudo).map_err(|e| (e, None))?;
    let status = cmd
        .status()
        .map_err(|e| (format!("failed to execute systemctl: {e}"), None))?;
    if status.success() {
        Ok(())
    } else {
        Err((
            format!("systemctl {} exited with status {status}", args.join(" ")),
            status.code().map(|c| ExitCode::from(c as u8)),
        ))
    }
}

fn run_systemctl_quiet(
    scope: UnitScope,
    args: &[&str],
    no_sudo: bool,
) -> Result<(), String> {
    let mut cmd = build_systemctl_cmd(scope, args, no_sudo)?;
    let status = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl exited with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_unit_names_strictly() {
        assert!(is_valid_unit_name("atriumd@miku.service"));
        assert!(is_valid_unit_name("atriumd.service"));
        assert!(!is_valid_unit_name(""));
        assert!(!is_valid_unit_name("../evil.service"));
        assert!(!is_valid_unit_name("foo/bar.service"));
    }

    #[test]
    fn renders_notify_and_watchdog_units() {
        let sys = render_system_template_unit(Path::new("/usr/local/bin/atriumd")).unwrap();
        assert!(sys.contains("Type=notify"));
        assert!(sys.contains("WatchdogSec=30s"));
        assert!(sys.contains("RuntimeDirectory=atrium"));
        assert!(sys.contains("StateDirectory=atrium"));
        assert!(sys.contains(
            "ExecStart=/usr/local/bin/atriumd --allow-user %i --socket /run/atrium/atriumd.sock"
        ));

        let usr = render_user_service_unit(Path::new("/opt/my dir/atriumd")).unwrap();
        assert!(usr.contains("Type=notify"));
        assert!(usr.contains("ExecStart=\"/opt/my dir/atriumd\" --socket %t/atrium/atriumd.sock"));
    }
}
