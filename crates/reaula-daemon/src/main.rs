//! `reaulad` — secure userspace driver daemon owning USB HID access to Aula keyboards.

mod audio;
mod coordinator;
mod evdev;
mod keyboards;
mod server;
mod shortcuts;
mod systemd;
mod uinput;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use reaula_core::ipc;

#[derive(Parser)]
#[command(
    name = "reaulad",
    version,
    about = "Secure hardware daemon owning USB HID access to indexed Aula keyboards"
)]
struct Args {
    /// Custom Unix domain socket path
    #[arg(long)]
    socket: Option<PathBuf>,

    /// Explicit indexed Aula keyboard model slug (e.g. win60he); auto-detects when omitted
    #[arg(long)]
    keyboard: Option<String>,

    /// List all Aula keyboard drivers compiled into this daemon and exit
    #[arg(long)]
    list_keyboards: bool,

    /// Numeric UID allowed to connect (repeatable)
    #[arg(long = "allow-uid")]
    allow_uids: Vec<u32>,

    /// Username allowed to connect, resolved via getpwnam (repeatable)
    #[arg(long = "allow-user")]
    allow_users: Vec<String>,

    /// Also allow UID 0 (root) clients
    #[arg(long)]
    allow_root: bool,

    /// Spawn reaulad detached in the background and exit once the socket is ready
    #[arg(long)]
    detach: bool,

    /// Custom JSON file path for memorizing daemon shortcut/Fn rules
    #[arg(long)]
    shortcuts_file: Option<PathBuf>,

    /// Internal privilege-dropped ALSA PCM capture helper mode
    #[arg(long, hide = true)]
    capture_pcm: bool,

    #[arg(long, hide = true, default_value = "monitor")]
    pcm_source: String,

    #[arg(long, hide = true)]
    pcm_device: Option<String>,

    #[arg(long, hide = true, default_value_t = 48000)]
    pcm_rate: u32,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let notify_on_err = !args.capture_pcm && !args.detach && !args.list_keyboards;
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if notify_on_err {
                systemd::SystemdNotifier::from_env().notify_errno(libc::EIO, &e);
            }
            eprintln!("reaulad error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn list_indexed_keyboards() {
    println!("Indexed Aula keyboard drivers ({}):", keyboards::DRIVERS.len());
    for entry in keyboards::DRIVERS {
        let s = entry.spec;
        let usb = s.primary_usb_id();
        println!(
            "  {:<10} {:<18} {:<10} {:04X}:{:04X} usage=0x{:04X} ({})",
            s.id,
            s.display_name,
            s.form_factor,
            usb.vid,
            usb.pid,
            s.vendor_usage_page,
            s.family.label(),
        );
    }
}

fn resolve_allowed_uids(args: &Args) -> Result<Vec<u32>, String> {
    let mut set = BTreeSet::new();
    for &uid in &args.allow_uids {
        set.insert(uid);
    }
    for user in &args.allow_users {
        let (uid, _) = ipc::lookup_user(user)
            .ok_or_else(|| format!("unknown local username '{user}'"))?;
        set.insert(uid);
    }
    if args.allow_root {
        set.insert(0);
    }

    if set.is_empty() {
        let me = ipc::current_uid();
        if me == 0 {
            if let Ok(sudo_uid_str) = std::env::var("SUDO_UID")
                && let Ok(sudo_uid) = sudo_uid_str.parse::<u32>()
                && sudo_uid != 0
            {
                set.insert(sudo_uid);
            } else {
                return Err(
                    "refusing to start as root without an explicit client allowlist; \
                     pass --allow-user <USER> or --allow-uid <UID> (or --allow-root)"
                        .into(),
                );
            }
        } else {
            set.insert(me);
        }
    }

    Ok(set.into_iter().collect())
}

fn run(args: Args) -> Result<(), String> {
    if args.capture_pcm {
        return audio::run_pcm_capture_helper(
            &args.pcm_source,
            args.pcm_device.as_deref(),
            args.pcm_rate,
        );
    }

    if args.list_keyboards {
        list_indexed_keyboards();
        return Ok(());
    }

    if let Some(ref kb) = args.keyboard
        && keyboards::find_entry(kb).is_none()
    {
        let known: Vec<&str> = keyboards::DRIVERS.iter().map(|e| e.spec.id).collect();
        return Err(format!(
            "unknown --keyboard '{kb}' (available: {})",
            known.join(", ")
        ));
    }

    let allowed_uids = resolve_allowed_uids(&args)?;
    let socket_path = args
        .socket
        .unwrap_or_else(ipc::default_daemon_socket_path);

    if args.detach {
        return server::spawn_detached(
            &socket_path,
            &allowed_uids,
            args.shortcuts_file.as_deref(),
            args.keyboard.as_deref(),
        );
    }

    server::run_server(
        socket_path,
        allowed_uids,
        args.shortcuts_file,
        args.keyboard,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_allow_uid_and_user() {
        let args = Args::try_parse_from(["reaulad", "--allow-uid", "1000", "--allow-root"]).unwrap();
        let uids = resolve_allowed_uids(&args).unwrap();
        assert_eq!(uids, vec![0, 1000]);
    }

    #[test]
    fn rejects_unknown_username() {
        let args =
            Args::try_parse_from(["reaulad", "--allow-user", "definitely_missing_user_xyz"]).unwrap();
        assert!(resolve_allowed_uids(&args).is_err());
    }

    #[test]
    fn rejects_unknown_keyboard_selector() {
        let args = Args::try_parse_from(["reaulad", "--keyboard", "does_not_exist"]).unwrap();
        let err = run(args).unwrap_err();
        assert!(err.contains("unknown --keyboard"));
    }
}
