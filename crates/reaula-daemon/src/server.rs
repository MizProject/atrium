//! Fail-closed Unix domain socket IPC server, client loop, and daemon detach support.

use std::fs;
use std::io::{self, BufReader};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use reaula_core::ipc::{self, DaemonRequest, DaemonResponse, IpcError, PeerCredentials};
use reaula_core::protocol::{CalibrationProgress, CalibrationStatus, LiveTravelSample};

use crate::coordinator::{CoordinatorMsg, DaemonRuntimeHealth, hardware_loop};
use crate::shortcuts::resolve_shortcuts_path;
use crate::systemd::{self, SystemdNotifier};

pub(crate) fn spawn_detached(
    socket_path: &Path,
    allowed_uids: &[u32],
    shortcuts_file: Option<&Path>,
    preferred_keyboard: Option<&str>,
) -> Result<(), String> {
    if let Ok(meta) = fs::symlink_metadata(socket_path) {
        if meta.file_type().is_socket() {
            if UnixStream::connect(socket_path).is_ok() {
                return Err(format!(
                    "reaulad is already running on {}",
                    socket_path.display()
                ));
            }
            let _ = fs::remove_file(socket_path);
        } else {
            return Err(format!(
                "refusing to overwrite non-socket path {}",
                socket_path.display()
            ));
        }
    }

    let exe = std::env::current_exe()
        .map_err(|e| format!("failed to resolve reaulad executable: {e}"))?;
    let mut cmd = Command::new(exe);
    cmd.arg("--socket").arg(socket_path);
    if let Some(sf) = shortcuts_file {
        cmd.arg("--shortcuts-file").arg(sf);
    }
    if let Some(kb) = preferred_keyboard {
        cmd.arg("--keyboard").arg(kb);
    }
    for &uid in allowed_uids {
        if uid == 0 {
            cmd.arg("--allow-root");
        } else {
            cmd.arg("--allow-uid").arg(uid.to_string());
        }
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    systemd::scrub_child_env(&mut cmd);

    // SAFETY: `setsid()` is async-signal-safe and detaches the child into a new session.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn detached reaulad: {e}"))?;

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("reaulad exited immediately with status {status}"));
        }
        if socket_path.exists() {
            println!(
                "reaulad started in background (pid={}, socket={}, allowed_uids={:?})",
                child.id(),
                socket_path.display(),
                allowed_uids
            );
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }

    Err(format!(
        "timed out waiting for reaulad socket at {}",
        socket_path.display()
    ))
}

pub(crate) fn run_server(
    mut socket_path: PathBuf,
    allowed_uids: Vec<u32>,
    shortcuts_file: Option<PathBuf>,
    preferred_keyboard: Option<String>,
) -> Result<(), String> {
    let mut notifier = SystemdNotifier::from_env();
    let (listener, socket_activated) = if let Some(activated) = systemd::take_systemd_socket()? {
        if let Some(bound) = activated.bound_path {
            socket_path = bound;
        }
        (activated.listener, true)
    } else {
        let l = ipc::bind_secure_listener(&socket_path, &allowed_uids)
            .map_err(|e| format!("failed to bind secure socket {}: {e}", socket_path.display()))?;
        (l, false)
    };
    let shortcuts_path = resolve_shortcuts_path(shortcuts_file.as_deref(), &socket_path);
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("failed to set socket nonblocking: {e}"))?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_sig = Arc::clone(&shutdown);
    ctrlc::set_handler(move || {
        shutdown_sig.store(true, Ordering::Relaxed);
    })
    .map_err(|e| format!("failed to install signal handler: {e}"))?;

    let health = Arc::new(DaemonRuntimeHealth::new(notifier.is_systemd()));
    let (tx, rx) = mpsc::channel::<CoordinatorMsg>();
    let hw_shutdown = Arc::clone(&shutdown);
    let hw_allowed = allowed_uids.clone();
    let hw_sock_display = socket_path.display().to_string();
    let hw_keyboard = preferred_keyboard.clone();
    let hw_health = Arc::clone(&health);
    let hw_handle = thread::Builder::new()
        .name("reaulad-hw".into())
        .spawn(move || {
            hardware_loop(
                rx,
                hw_shutdown,
                hw_allowed,
                hw_sock_display,
                shortcuts_path,
                hw_keyboard,
                hw_health,
            )
        })
        .map_err(|e| format!("failed to spawn hardware worker: {e}"))?;

    let mut last_connected = health.is_device_connected();
    let ready_status = format!(
        "Listening on {} ({})",
        socket_path.display(),
        if socket_activated {
            "systemd socket-activated"
        } else {
            "ready"
        }
    );
    notifier.notify_ready(&ready_status);

    eprintln!(
        "reaulad listening on {} (pid={}, uid={}, allowed_uids={:?}, keyboard={}, systemd={})",
        socket_path.display(),
        std::process::id(),
        ipc::current_uid(),
        allowed_uids,
        preferred_keyboard.as_deref().unwrap_or("auto"),
        notifier.is_systemd(),
    );

    let mut next_client_id = 1u64;
    let allowed_arc = Arc::new(allowed_uids);

    while !shutdown.load(Ordering::Relaxed) {
        let connected_now = health.is_device_connected();
        if connected_now != last_connected {
            last_connected = connected_now;
            let st = if connected_now {
                format!("Keyboard connected (socket {})", socket_path.display())
            } else {
                format!(
                    "Waiting for USB HID keyboard (socket {})",
                    socket_path.display()
                )
            };
            notifier.notify_status(&st);
        }

        if let Some(wd_interval) = notifier.watchdog_interval() {
            // Only ping systemd watchdog if hardware_loop heartbeat is still advancing.
            if health.heartbeat_age() <= wd_interval.saturating_mul(2) {
                notifier.maybe_notify_watchdog();
            }
        }

        match listener.accept() {
            Ok((stream, _addr)) => {
                let cred = match ipc::peer_credentials(&stream) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("rejected connection without SO_PEERCRED: {e}");
                        continue;
                    }
                };
                if !ipc::is_peer_authorized(&cred, &allowed_arc) {
                    eprintln!(
                        "SECURITY: rejected unauthorized peer uid={} gid={} pid={}",
                        cred.uid, cred.gid, cred.pid
                    );
                    continue;
                }

                let client_id = next_client_id;
                next_client_id += 1;
                let tx_client = tx.clone();
                let shutdown_client = Arc::clone(&shutdown);
                let _ = thread::Builder::new()
                    .name(format!("reaulad-client-{client_id}"))
                    .spawn(move || {
                        handle_client(client_id, cred, stream, tx_client, shutdown_client);
                    });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                eprintln!("accept error: {e}");
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    notifier.notify_stopping("Shutting down reaulad cleanly");
    drop(listener);
    if !socket_activated {
        let _ = fs::remove_file(&socket_path);
    }
    let _ = tx.send(CoordinatorMsg::RequestShutdown);
    let _ = hw_handle.join();
    eprintln!("reaulad shut down cleanly.");
    Ok(())
}

fn handle_client(
    client_id: u64,
    cred: PeerCredentials,
    stream: UnixStream,
    coord_tx: Sender<CoordinatorMsg>,
    shutdown: Arc<AtomicBool>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let Ok(writer_base) = stream.try_clone() else {
        return;
    };
    let mut writer = writer_base;
    let mut reader = BufReader::new(stream);

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        let req: DaemonRequest = match ipc::read_json_line(&mut reader) {
            Ok(r) => r,
            Err(IpcError::Disconnected) => break,
            Err(IpcError::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(e @ IpcError::Json(_)) => {
                eprintln!(
                    "client #{client_id} (uid={}, pid={}) request decode error: {e}",
                    cred.uid, cred.pid
                );
                if ipc::write_json_line(
                    &mut writer,
                    &DaemonResponse::Error {
                        message: e.to_string(),
                    },
                )
                .is_err()
                {
                    break;
                }
                continue;
            }
            Err(e) => {
                eprintln!(
                    "client #{client_id} (uid={}, pid={}) read error: {e}",
                    cred.uid, cred.pid
                );
                let _ = ipc::write_json_line(
                    &mut writer,
                    &DaemonResponse::Error {
                        message: e.to_string(),
                    },
                );
                break;
            }
        };

        match req {
            DaemonRequest::SubscribeTravel { indices } => {
                let (reply_tx, reply_rx) = mpsc::channel();
                if coord_tx
                    .send(CoordinatorMsg::SubscribeTravel {
                        client_id,
                        indices,
                        reply_tx,
                    })
                    .is_err()
                {
                    break;
                }
                match reply_rx.recv() {
                    Ok(Ok((unit_mm, sample_rx))) => {
                        if ipc::write_json_line(
                            &mut writer,
                            &DaemonResponse::TravelSubscribed { unit_mm },
                        )
                        .is_err()
                        {
                            let _ = coord_tx.send(CoordinatorMsg::UnsubscribeTravel { client_id });
                            break;
                        }
                        stream_travel_loop(
                            client_id,
                            &mut reader,
                            &mut writer,
                            &sample_rx,
                            &coord_tx,
                            &shutdown,
                        );
                        let _ = coord_tx.send(CoordinatorMsg::UnsubscribeTravel { client_id });
                    }
                    Ok(Err(message)) => {
                        let _ = ipc::write_json_line(&mut writer, &DaemonResponse::Error { message });
                    }
                    Err(_) => break,
                }
            }
            DaemonRequest::UnsubscribeTravel => {
                let _ = coord_tx.send(CoordinatorMsg::UnsubscribeTravel { client_id });
                let _ = ipc::write_json_line(&mut writer, &DaemonResponse::Ack);
            }
            DaemonRequest::StartCalibration { any_key } => {
                let (reply_tx, reply_rx) = mpsc::channel();
                if coord_tx
                    .send(CoordinatorMsg::StartCalibration {
                        client_id,
                        any_key,
                        reply_tx,
                    })
                    .is_err()
                {
                    break;
                }
                match reply_rx.recv() {
                    Ok(Ok(cal_rx)) => {
                        if ipc::write_json_line(&mut writer, &DaemonResponse::CalibrationStarted)
                            .is_err()
                        {
                            let _ = coord_tx.send(CoordinatorMsg::StopCalibration { client_id });
                            break;
                        }
                        stream_calibration_loop(
                            &mut reader,
                            &mut writer,
                            &cal_rx,
                            &shutdown,
                        );
                        let _ = coord_tx.send(CoordinatorMsg::StopCalibration { client_id });
                    }
                    Ok(Err(message)) => {
                        let _ = ipc::write_json_line(&mut writer, &DaemonResponse::Error { message });
                    }
                    Err(_) => break,
                }
            }
            DaemonRequest::StopCalibration => {
                let _ = coord_tx.send(CoordinatorMsg::StopCalibration { client_id });
                let _ = ipc::write_json_line(&mut writer, &DaemonResponse::Ack);
            }
            other => {
                let is_shutdown = matches!(other, DaemonRequest::Shutdown);
                let (reply_tx, reply_rx) = mpsc::channel();
                if coord_tx
                    .send(CoordinatorMsg::Rpc {
                        req: other,
                        reply_tx,
                    })
                    .is_err()
                {
                    break;
                }
                let resp = match reply_rx.recv() {
                    Ok(r) => r,
                    Err(_) => DaemonResponse::Error {
                        message: "hardware worker terminated".into(),
                    },
                };
                if ipc::write_json_line(&mut writer, &resp).is_err() {
                    break;
                }
                if is_shutdown {
                    shutdown.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }
    }
    let _ = coord_tx.send(CoordinatorMsg::UnsubscribeTravel { client_id });
    let _ = coord_tx.send(CoordinatorMsg::StopCalibration { client_id });
}

fn stream_travel_loop(
    _client_id: u64,
    reader: &mut BufReader<UnixStream>,
    writer: &mut UnixStream,
    sample_rx: &Receiver<(LiveTravelSample, f64)>,
    _coord_tx: &Sender<CoordinatorMsg>,
    daemon_shutdown: &Arc<AtomicBool>,
) {
    let stop_stream = Arc::new(AtomicBool::new(false));
    let _ = reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(20)));

    while !daemon_shutdown.load(Ordering::Relaxed) && !stop_stream.load(Ordering::Relaxed) {
        match ipc::read_json_line::<_, DaemonRequest>(reader) {
            Ok(DaemonRequest::UnsubscribeTravel) => {
                let _ = ipc::write_json_line(writer, &DaemonResponse::Ack);
                break;
            }
            Ok(_) => {}
            Err(IpcError::Disconnected) => break,
            Err(IpcError::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }

        match sample_rx.recv_timeout(Duration::from_millis(20)) {
            Ok((sample, unit_mm)) => {
                if ipc::write_json_line(
                    writer,
                    &DaemonResponse::TravelSample { sample, unit_mm },
                )
                .is_err()
                {
                    break;
                }
                while let Ok((extra, u)) = sample_rx.try_recv() {
                    if ipc::write_json_line(
                        writer,
                        &DaemonResponse::TravelSample {
                            sample: extra,
                            unit_mm: u,
                        },
                    )
                    .is_err()
                    {
                        stop_stream.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(10)));
}

fn stream_calibration_loop(
    reader: &mut BufReader<UnixStream>,
    writer: &mut UnixStream,
    cal_rx: &Receiver<CalibrationProgress>,
    daemon_shutdown: &Arc<AtomicBool>,
) {
    let _ = reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(20)));

    while !daemon_shutdown.load(Ordering::Relaxed) {
        match ipc::read_json_line::<_, DaemonRequest>(reader) {
            Ok(DaemonRequest::StopCalibration) => {
                let _ = ipc::write_json_line(writer, &DaemonResponse::Ack);
                break;
            }
            Ok(_) => {}
            Err(IpcError::Disconnected) => break,
            Err(IpcError::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }

        match cal_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(prog) => {
                let terminal = !matches!(prog.status, CalibrationStatus::Progress);
                if ipc::write_json_line(writer, &DaemonResponse::CalibrationUpdate(prog)).is_err() {
                    break;
                }
                if terminal {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(10)));
}
