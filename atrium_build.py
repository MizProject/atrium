#!/usr/bin/env python3
"""Build, verification, packaging, checksum, and release-reporting tool for Atrium.

Copyright (C) 2026 MizProject Philippines (GNU AGPL-3.0-or-later).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shlex
import shutil
import subprocess
import sys
import tarfile
import time
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable

REPO_ROOT = Path(__file__).resolve().parent
BINARIES = ("atrium", "atriumd", "atrium-tui", "atrium-gui")
REQUIRED_PKG_CONFIG_DEPS = (
    ("libudev", "udev / hidraw enumeration"),
    ("libusb-1.0", "USB HID transport"),
    ("hidapi-hidraw", "hidapi kernel hidraw backend"),
    ("alsa", "ALSA PCM audio capture (libasound)"),
    ("fontconfig", "GUI system font resolution"),
    ("freetype2", "GUI glyph rasterization"),
    ("wayland-client", "GPUI Wayland windowing"),
    ("xkbcommon", "GPUI keyboard layout handling"),
    ("xcb", "GPUI X11 windowing fallback"),
)
TEST_SUMMARY_RE = re.compile(
    r"test result:\s+(\w+)\.\s+(\d+)\s+passed;\s+(\d+)\s+failed;\s+(\d+)\s+ignored"
)
TEST_BIN_RE = re.compile(r"Running\s+(?:unittests|tests)\s+.*?\(([^)]+)\)")
SEMVER_TAG_RE = re.compile(r"^v?\d+\.\d+\.\d+(?:\+[0-9A-Za-z.-]+)?$")
PRERELEASE_HINT_RE = re.compile(r"(alpha|beta|rc|pre|dev|nightly|snapshot)", re.IGNORECASE)


@dataclass
class StepRecord:
    name: str
    command: str
    status: str  # PASS | FAIL | SKIPPED | WARN
    duration_sec: float
    details: str = ""


@dataclass
class TestSuiteMetric:
    binary: str
    passed: int
    failed: int
    ignored: int
    status: str


@dataclass
class BinaryArtifactMetric:
    name: str
    raw_bytes: int
    packaged_bytes: int
    stripped: bool
    sha256: str


@dataclass
class EnvCheckItem:
    category: str
    name: str
    status: str  # OK | WARN | MISSING
    detail: str
    required: bool = True


def run_capture(
    cmd: list[str],
    *,
    cwd: Path = REPO_ROOT,
    echo: bool = True,
) -> tuple[int, str, float]:
    """Execute a subprocess streaming interleaved output while capturing combined logs."""
    t0 = time.monotonic()
    if echo:
        print(f"==> $ {shlex.join(cmd)}", flush=True)
    proc = subprocess.Popen(
        cmd,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    lines: list[str] = []
    assert proc.stdout is not None
    for line in proc.stdout:
        lines.append(line)
        if echo:
            sys.stdout.write(line)
            sys.stdout.flush()
    rc = proc.wait()
    return rc, "".join(lines), round(time.monotonic() - t0, 2)


def run_quiet(cmd: list[str], *, cwd: Path = REPO_ROOT) -> tuple[int, str]:
    try:
        res = subprocess.run(
            cmd,
            cwd=cwd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            check=False,
        )
        return res.returncode, res.stdout.strip()
    except OSError as exc:
        return 127, str(exc)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def human_size(size_bytes: int) -> str:
    val = float(size_bytes)
    for unit in ("B", "KiB", "MiB", "GiB"):
        if val < 1024.0 or unit == "GiB":
            return f"{int(val)} B" if unit == "B" else f"{val:.2f} {unit}"
        val /= 1024.0
    return f"{size_bytes} B"


def append_text_file(path_str: str | None, content: str) -> None:
    if not path_str:
        return
    p = Path(path_str)
    p.parent.mkdir(parents=True, exist_ok=True)
    with p.open("a", encoding="utf-8") as fh:
        fh.write(content)
        if not content.endswith("\n"):
            fh.write("\n")


def write_github_outputs(path_str: str | None, pairs: dict[str, str]) -> None:
    if not path_str:
        return
    p = Path(path_str)
    p.parent.mkdir(parents=True, exist_ok=True)
    with p.open("a", encoding="utf-8") as fh:
        for key, val in pairs.items():
            clean = str(val).replace("\r", " ").replace("\n", " ")
            fh.write(f"{key}={clean}\n")


def detect_workspace_version() -> tuple[str, list[dict[str, str]]]:
    """Read workspace crate versions via `cargo metadata` with Cargo.toml regex fallback."""
    rc, raw = run_quiet(["cargo", "metadata", "--no-deps", "--format-version", "1"])
    crates: list[dict[str, str]] = []
    if rc == 0 and raw:
        try:
            meta = json.loads(raw)
            for pkg in meta.get("packages", []):
                crates.append(
                    {"name": pkg.get("name", ""), "version": pkg.get("version", "0.1.0")}
                )
            for name in ("atrium-cli", "atrium-daemon", "atrium-core"):
                for item in crates:
                    if item["name"] == name and item["version"]:
                        return item["version"], crates
            if crates:
                return crates[0]["version"], crates
        except json.JSONDecodeError:
            pass

    for cargo_toml in sorted((REPO_ROOT / "crates").glob("*/Cargo.toml")):
        txt = cargo_toml.read_text(encoding="utf-8")
        m_name = re.search(r'^name\s*=\s*"([^"]+)"', txt, re.M)
        m_ver = re.search(r'^version\s*=\s*"([^"]+)"', txt, re.M)
        if m_name and m_ver:
            crates.append({"name": m_name.group(1), "version": m_ver.group(1)})
    version = next(
        (c["version"] for c in crates if c["name"] in ("atrium-cli", "atrium-core")),
        "0.1.0",
    )
    return version, crates


def git_info() -> dict[str, str]:
    def _git(*args: str, default: str = "") -> str:
        rc, out = run_quiet(["git", *args])
        return out if rc == 0 and out else default

    short_sha = _git("rev-parse", "--short", "HEAD", default="unknown")
    full_sha = _git("rev-parse", "HEAD", default="unknown")
    branch = os.environ.get("GITHUB_REF_NAME") or _git(
        "rev-parse", "--abbrev-ref", "HEAD", default="detached"
    )
    commit_count = _git("rev-list", "--count", "HEAD", default="0")
    exact_tag = _git("describe", "--tags", "--exact-match", default="")
    prev_tag = _git("describe", "--tags", "--abbrev=0", "HEAD^", default="")
    return {
        "short_sha": short_sha,
        "full_sha": full_sha,
        "branch": branch,
        "commit_count": commit_count,
        "exact_tag": exact_tag,
        "prev_tag": prev_tag,
    }


def resolve_bool_choice(mode: str, auto_value: bool) -> bool:
    norm = (mode or "auto").strip().lower()
    if norm == "true":
        return True
    if norm == "false":
        return False
    return auto_value


def cmd_metadata(args: argparse.Namespace) -> int:
    version, crates = detect_workspace_version()
    g = git_info()
    short_sha = g["short_sha"]
    build_number = os.environ.get("GITHUB_RUN_NUMBER") or g["commit_count"] or "1"
    event_name = os.environ.get("GITHUB_EVENT_NAME", "local")
    ref_type = os.environ.get("GITHUB_REF_TYPE", "")
    ref_name = os.environ.get("GITHUB_REF_NAME", "") or g["branch"]
    is_tag = ref_type == "tag" or (
        os.environ.get("GITHUB_REF", "").startswith("refs/tags/")
    ) or bool(g["exact_tag"] and event_name == "local")

    token_results: dict[str, bool] = {}
    for key_name in args.check_env_key:
        token_results[key_name] = bool(os.environ.get(key_name, "").strip())

    has_token_keys = token_results.get("TOKEN_KEYS", False)
    env_keys_present = all(token_results.values()) if token_results else True

    if args.release_tag:
        release_tag = args.release_tag.strip()
    elif is_tag:
        release_tag = ref_name if ref_name else g["exact_tag"]
    else:
        release_tag = f"v{version}-pre.{build_number}.{short_sha}"

    auto_prerelease = True
    if is_tag:
        tag_str = release_tag
        auto_prerelease = bool(
            PRERELEASE_HINT_RE.search(tag_str) or not SEMVER_TAG_RE.match(tag_str)
        )
    prerelease = resolve_bool_choice(args.prerelease, auto_prerelease)

    is_pr = event_name in ("pull_request", "pull_request_target")
    auto_publish = (not is_pr) and env_keys_present
    should_release = resolve_bool_choice(args.publish_release, auto_publish)

    if (args.publish_release or "").strip().lower() == "true" and not env_keys_present:
        print(
            "::error::publish_release=true requested, but required token env var(s) are empty.",
            file=sys.stderr,
        )
        should_release = False

    kind_label = "Pre-release" if prerelease else "Release"
    release_name = f"Atrium {release_tag} ({kind_label})"
    dynamic_version = f"{version}+b{build_number}.{short_sha}"
    target_arch = platform.machine() or "x86_64"

    outputs = {
        "version": version,
        "build_number": str(build_number),
        "dynamic_version": dynamic_version,
        "short_sha": short_sha,
        "full_sha": g["full_sha"],
        "arch": target_arch,
        "release_tag": release_tag,
        "release_name": release_name,
        "prerelease": "true" if prerelease else "false",
        "should_release": "true" if should_release else "false",
        "has_token_keys": "true" if has_token_keys else "false",
        "env_keys_present": "true" if env_keys_present else "false",
    }

    print(json.dumps({"metadata": outputs, "crates": crates}, indent=2))
    write_github_outputs(args.github_output, outputs)

    summary_lines = [
        "### Metadata & Release Resolution",
        "",
        "| Field | Value |",
        "| :--- | :--- |",
        f"| Workspace Version | `{version}` (`{dynamic_version}`) |",
        f"| Architecture | `linux-{target_arch}` |",
        f"| Git Event / Ref | `{event_name}` (`{ref_name}` @ `{short_sha}`) |",
        f"| Release Tag | `{release_tag}` |",
        f"| Release Type | **{kind_label}** (`prerelease={outputs['prerelease']}`) |",
        f"| Publish Release | `{outputs['should_release']}` (`TOKEN_KEYS` present: `{outputs['has_token_keys']}`) |",
        "",
    ]
    append_text_file(args.step_summary, "\n".join(summary_lines))
    return 0


def collect_env_checks() -> list[EnvCheckItem]:
    checks: list[EnvCheckItem] = []

    checks.append(
        EnvCheckItem(
            category="Host",
            name="OS / Arch",
            status="OK",
            detail=f"{platform.system()} {platform.release()} ({platform.machine()})",
        )
    )
    checks.append(
        EnvCheckItem(
            category="Host",
            name="Python",
            status="OK",
            detail=platform.python_version(),
        )
    )

    rc, rustc_ver = run_quiet(["rustc", "--version"])
    rustc_ok = rc == 0
    if rustc_ok:
        m = re.search(r"(\d+)\.(\d+)\.(\d+)", rustc_ver)
        if m and (int(m.group(1)), int(m.group(2))) < (1, 85):
            checks.append(
                EnvCheckItem(
                    "Toolchain",
                    "rustc (>= 1.85 for Edition 2024)",
                    "MISSING",
                    f"{rustc_ver} is too old",
                )
            )
        else:
            checks.append(EnvCheckItem("Toolchain", "rustc", "OK", rustc_ver))
    else:
        checks.append(EnvCheckItem("Toolchain", "rustc", "MISSING", rustc_ver))

    for tool, req in (
        ("cargo", True),
        ("pkg-config", True),
        ("git", True),
        ("tar", True),
        ("sha256sum", True),
        ("strip", False),
        ("systemctl", False),
    ):
        rc_t, out_t = run_quiet([tool, "--version"])
        first_line = out_t.splitlines()[0] if out_t else "unavailable"
        if rc_t == 0:
            checks.append(EnvCheckItem("Tool", tool, "OK", first_line, required=req))
        else:
            checks.append(
                EnvCheckItem(
                    "Tool",
                    tool,
                    "MISSING" if req else "WARN",
                    first_line,
                    required=req,
                )
            )

    for subcmd, label in (("clippy", "cargo clippy"), ("fmt", "cargo fmt")):
        rc_c, out_c = run_quiet(["cargo", subcmd, "--version"])
        if rc_c == 0:
            checks.append(
                EnvCheckItem("Toolchain", label, "OK", out_c.splitlines()[0], required=False)
            )
        else:
            checks.append(
                EnvCheckItem("Toolchain", label, "WARN", "component not installed", required=False)
            )

    for pkg, desc in REQUIRED_PKG_CONFIG_DEPS:
        rc_p, modver = run_quiet(["pkg-config", "--modversion", pkg])
        if rc_p == 0:
            checks.append(
                EnvCheckItem("Native Lib", f"{pkg} ({desc})", "OK", f"version {modver}")
            )
        else:
            checks.append(
                EnvCheckItem(
                    "Native Lib",
                    f"{pkg} ({desc})",
                    "MISSING",
                    "not found via pkg-config",
                )
            )

    return checks


def cmd_check(args: argparse.Namespace) -> int:
    checks = collect_env_checks()
    missing_req = [c for c in checks if c.status == "MISSING" and c.required]

    print("=== Environment Verification ===")
    for item in checks:
        print(f"  [{item.status:<7}] {item.category:<10} {item.name}: {item.detail}")

    summary_lines = [
        "### Environment Check",
        "",
        "| Category | Dependency | Status | Detail |",
        "| :--- | :--- | :--- | :--- |",
    ]
    for item in checks:
        badge = "✅ OK" if item.status == "OK" else ("⚠️ WARN" if item.status == "WARN" else "❌ MISSING")
        summary_lines.append(f"| {item.category} | `{item.name}` | {badge} | {item.detail} |")
    summary_lines.append("")
    append_text_file(args.step_summary, "\n".join(summary_lines))

    if missing_req:
        print(
            f"\nERROR: {len(missing_req)} required build dependencies missing.",
            file=sys.stderr,
        )
        return 1
    return 0


def parse_cargo_tests(combined_output: str) -> list[TestSuiteMetric]:
    metrics: list[TestSuiteMetric] = []
    current_bin = "workspace"
    for line in combined_output.splitlines():
        m_bin = TEST_BIN_RE.search(line)
        if m_bin:
            path_part = m_bin.group(1)
            current_bin = re.sub(r"-[0-9a-f]{16}$", "", Path(path_part).name)
            continue
        m_doc = re.search(r"Doc-tests\s+([^\s]+)", line)
        if m_doc:
            current_bin = f"doctests:{m_doc.group(1)}"
            continue
        m_res = TEST_SUMMARY_RE.search(line)
        if m_res:
            metrics.append(
                TestSuiteMetric(
                    binary=current_bin,
                    status=m_res.group(1),
                    passed=int(m_res.group(2)),
                    failed=int(m_res.group(3)),
                    ignored=int(m_res.group(4)),
                )
            )
    return metrics


def stage_release_artifacts(
    *,
    profile: str,
    target_triple: str | None,
    dist_dir: Path,
    release_dir: Path,
    release_tag: str,
    strip_bins: bool,
    steps: list[StepRecord],
    test_metrics: list[TestSuiteMetric],
) -> tuple[list[BinaryArtifactMetric], Path]:
    t0 = time.monotonic()
    arch = platform.machine() or "x86_64"
    target_subdir = Path(target_triple) / profile if target_triple else Path(profile)
    cargo_bin_dir = REPO_ROOT / "target" / target_subdir

    if dist_dir.exists():
        shutil.rmtree(dist_dir)
    release_dir.mkdir(parents=True, exist_ok=True)

    bundle_name = f"atrium-{release_tag}-linux-{arch}"
    bundle_root = dist_dir / bundle_name
    bin_out = bundle_root / "bin"
    systemd_out = bundle_root / "systemd"
    udev_out = bundle_root / "udev"
    bin_out.mkdir(parents=True, exist_ok=True)
    systemd_out.mkdir(parents=True, exist_ok=True)
    udev_out.mkdir(parents=True, exist_ok=True)

    has_strip = strip_bins and shutil.which("strip") is not None
    binary_metrics: list[BinaryArtifactMetric] = []

    for name in BINARIES:
        src = cargo_bin_dir / name
        if not src.is_file():
            raise FileNotFoundError(f"Expected compiled binary missing: {src}")
        raw_bytes = src.stat().st_size
        dst = bin_out / name
        shutil.copy2(src, dst)
        os.chmod(dst, 0o755)
        did_strip = False
        if has_strip and profile == "release":
            rc_s, _ = run_quiet(["strip", "--strip-unneeded", str(dst)])
            did_strip = rc_s == 0
        pkg_bytes = dst.stat().st_size
        digest = sha256_file(dst)
        binary_metrics.append(
            BinaryArtifactMetric(
                name=name,
                raw_bytes=raw_bytes,
                packaged_bytes=pkg_bytes,
                stripped=did_strip,
                sha256=digest,
            )
        )

    for src_dir, dst_dir in (
        (REPO_ROOT / "systemd", systemd_out),
        (REPO_ROOT / "udev", udev_out),
    ):
        if src_dir.is_dir():
            for item in sorted(src_dir.iterdir()):
                if item.is_file():
                    shutil.copy2(item, dst_dir / item.name)

    for doc_name in ("LICENSE", "README.md", "CONTRIBUTING.md"):
        doc_path = REPO_ROOT / doc_name
        if doc_path.is_file():
            shutil.copy2(doc_path, bundle_root / doc_name)

    icon_src = REPO_ROOT / "images" / "icon" / "atrium.png"
    if icon_src.is_file():
        assets_dir = bundle_root / "assets"
        assets_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(icon_src, assets_dir / "atrium.png")

    g = git_info()
    version, _ = detect_workspace_version()
    build_info = (
        f"Atrium {release_tag}\n"
        f"version: {version}\n"
        f"commit: {g['full_sha']}\n"
        f"profile: {profile}\n"
        f"arch: linux-{arch}\n"
        f"built_at_utc: {datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')}\n"
    )
    (bundle_root / "BUILD_INFO.txt").write_text(build_info, encoding="utf-8")

    archive_path = release_dir / f"{bundle_name}.tar.gz"
    if archive_path.exists():
        archive_path.unlink()
    with tarfile.open(archive_path, "w:gz") as tar:
        tar.add(bundle_root, arcname=bundle_name)

    dur = round(time.monotonic() - t0, 2)
    steps.append(
        StepRecord(
            name="Stage & Package",
            command=f"tar -czf {archive_path.name} {bundle_name}/",
            status="PASS",
            duration_sec=dur,
            details=f"{len(binary_metrics)} binaries + systemd/udev -> {human_size(archive_path.stat().st_size)}",
        )
    )

    report_payload = {
        "version": version,
        "release_tag": release_tag,
        "profile": profile,
        "arch": f"linux-{arch}",
        "git": g,
        "generated_at_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "steps": [asdict(s) for s in steps],
        "tests": [asdict(t) for t in test_metrics],
        "binaries": [asdict(b) for b in binary_metrics],
        "archive": {
            "name": archive_path.name,
            "bytes": archive_path.stat().st_size,
            "sha256": sha256_file(archive_path),
        },
    }
    (dist_dir / "build-report.json").write_text(
        json.dumps(report_payload, indent=2), encoding="utf-8"
    )
    return binary_metrics, archive_path


def write_checksums(release_dir: Path) -> Path:
    entries: list[str] = []
    for f in sorted(release_dir.iterdir(), key=lambda p: p.name):
        if f.is_file() and f.name != "SHA256SUMS.txt":
            entries.append(f"{sha256_file(f)}  {f.name}")
    if not entries:
        raise FileNotFoundError(f"No release files found in {release_dir}")
    sums_path = release_dir / "SHA256SUMS.txt"
    sums_path.write_text("\n".join(entries) + "\n", encoding="utf-8")
    return sums_path


def format_build_summary_markdown(
    *,
    title: str,
    steps: Iterable[StepRecord],
    tests: Iterable[TestSuiteMetric],
    binaries: Iterable[BinaryArtifactMetric],
    archive: Path | None,
) -> str:
    lines = [f"### {title}", ""]
    lines.extend(
        [
            "#### Process & Stage Report",
            "",
            "| Stage | Status | Duration | Details |",
            "| :--- | :--- | :--- | :--- |",
        ]
    )
    for st in steps:
        icon = {"PASS": "✅ PASS", "FAIL": "❌ FAIL", "WARN": "⚠️ WARN"}.get(
            st.status, f"⏭️ {st.status}"
        )
        lines.append(
            f"| {st.name} | {icon} | {st.duration_sec:.2f}s | {st.details or f'`{st.command}`'} |"
        )
    lines.append("")

    test_list = list(tests)
    if test_list:
        total_p = sum(t.passed for t in test_list)
        total_f = sum(t.failed for t in test_list)
        total_i = sum(t.ignored for t in test_list)
        lines.extend(
            [
                f"#### Test Suite Breakdown ({total_p} passed, {total_f} failed, {total_i} ignored)",
                "",
                "| Test Target | Passed | Failed | Ignored | Result |",
                "| :--- | ---: | ---: | ---: | :--- |",
            ]
        )
        for tm in test_list:
            lines.append(
                f"| `{tm.binary}` | {tm.passed} | {tm.failed} | {tm.ignored} | `{tm.status}` |"
            )
        lines.append("")

    bin_list = list(binaries)
    if bin_list:
        lines.extend(
            [
                "#### Compiled Binaries",
                "",
                "| Binary | Raw Size | Packaged Size | Stripped | SHA-256 (short) |",
                "| :--- | ---: | ---: | :--- | :--- |",
            ]
        )
        for bm in bin_list:
            lines.append(
                f"| `{bm.name}` | {human_size(bm.raw_bytes)} | {human_size(bm.packaged_bytes)} "
                f"| {'yes' if bm.stripped else 'no'} | `{bm.sha256[:16]}...` |"
            )
        lines.append("")

    if archive and archive.is_file():
        lines.extend(
            [
                "#### Packaged Release Archive",
                "",
                f"- `{archive.name}` — {human_size(archive.stat().st_size)}",
                "",
            ]
        )
    return "\n".join(lines)


def cmd_build(args: argparse.Namespace) -> int:
    version, _ = detect_workspace_version()
    g = git_info()
    build_num = os.environ.get("GITHUB_RUN_NUMBER") or g["commit_count"] or "1"
    release_tag = args.release_tag or g["exact_tag"] or f"v{version}-pre.{build_num}.{g['short_sha']}"

    dist_dir = Path(args.dist_dir).resolve()
    release_dir = Path(args.release_dir).resolve()
    steps: list[StepRecord] = []
    tests: list[TestSuiteMetric] = []
    binaries: list[BinaryArtifactMetric] = []
    archive: Path | None = None

    cargo_common_flags: list[str] = []
    if args.locked:
        cargo_common_flags.append("--locked")
    if args.target:
        cargo_common_flags.extend(["--target", args.target])
    if args.jobs:
        cargo_common_flags.extend(["--jobs", str(args.jobs)])

    def _run_stage(name: str, cmd: list[str]) -> tuple[int, str]:
        rc, out, dur = run_capture(cmd)
        status = "PASS" if rc == 0 else "FAIL"
        detail = "" if rc == 0 else f"exited with code {rc}"
        steps.append(
            StepRecord(
                name=name,
                command=shlex.join(cmd),
                status=status,
                duration_sec=dur,
                details=detail,
            )
        )
        return rc, out

    overall_rc = 0
    try:
        if args.clean:
            rc, _ = _run_stage("Workspace Clean", ["cargo", "clean"])
            if rc != 0:
                overall_rc = rc
                raise RuntimeError("cargo clean failed")

        if args.fmt_check:
            rc, _ = _run_stage("Rustfmt Check", ["cargo", "fmt", "--all", "--", "--check"])
            if rc != 0:
                overall_rc = rc
                raise RuntimeError("cargo fmt check failed")
        else:
            steps.append(
                StepRecord("Rustfmt Check", "cargo fmt --all -- --check", "SKIPPED", 0.0, "disabled")
            )

        if not args.skip_clippy:
            cmd = ["cargo", "clippy", "--workspace", "--all-targets", *cargo_common_flags, "--", "-D", "warnings"]
            rc, _ = _run_stage("Workspace Clippy", cmd)
            if rc != 0:
                overall_rc = rc
                raise RuntimeError("cargo clippy failed")
        else:
            steps.append(StepRecord("Workspace Clippy", "cargo clippy ...", "SKIPPED", 0.0, "--skip-clippy"))

        if not args.skip_tests:
            cmd = ["cargo", "test", "--workspace", *cargo_common_flags]
            rc, out = _run_stage("Workspace Tests", cmd)
            tests = parse_cargo_tests(out)
            if tests and steps:
                steps[-1].details = (
                    f"{sum(t.passed for t in tests)} passed, "
                    f"{sum(t.failed for t in tests)} failed, "
                    f"{sum(t.ignored for t in tests)} ignored"
                )
            if rc != 0:
                overall_rc = rc
                raise RuntimeError("cargo test failed")
        else:
            steps.append(StepRecord("Workspace Tests", "cargo test --workspace", "SKIPPED", 0.0, "--skip-tests"))

        build_cmd = ["cargo", "build", "--workspace", "--bins", *cargo_common_flags]
        if args.profile == "release":
            build_cmd.append("--release")
        rc, _ = _run_stage(f"Binary Build ({args.profile})", build_cmd)
        if rc != 0:
            overall_rc = rc
            raise RuntimeError("cargo build failed")

        binaries, archive = stage_release_artifacts(
            profile=args.profile,
            target_triple=args.target,
            dist_dir=dist_dir,
            release_dir=release_dir,
            release_tag=release_tag,
            strip_bins=not args.no_strip,
            steps=steps,
            test_metrics=tests,
        )
        sums = write_checksums(release_dir)
        print(f"==> Wrote checksums: {sums}")
    except Exception as exc:  # noqa: BLE001
        print(f"ERROR: {exc}", file=sys.stderr)
        overall_rc = overall_rc or 1
    finally:
        title = (
            f"✅ Atrium Build Succeeded (`{release_tag}` / `{args.profile}`)"
            if overall_rc == 0
            else f"❌ Atrium Build Failed (`{release_tag}` / `{args.profile}`)"
        )
        md = format_build_summary_markdown(
            title=title,
            steps=steps,
            tests=tests,
            binaries=binaries,
            archive=archive,
        )
        append_text_file(args.step_summary, md)
        if overall_rc == 0:
            print("\n" + md)

    return overall_rc


def cmd_checksums(args: argparse.Namespace) -> int:
    rel_dir = Path(args.release_dir).resolve()
    sums_path = write_checksums(rel_dir)
    print(f"Refreshed {sums_path}:")
    print(sums_path.read_text(encoding="utf-8"), end="")
    return 0


def cmd_report(args: argparse.Namespace) -> int:
    rel_dir = Path(args.release_dir).resolve()
    is_pre = str(args.prerelease).strip().lower() == "true"
    kind = "Pre-release" if is_pre else "Release"
    g = git_info()

    report_data: dict = {}
    if args.report_json and Path(args.report_json).is_file():
        report_data = json.loads(Path(args.report_json).read_text(encoding="utf-8"))

    assets: list[tuple[str, int, str]] = []
    for path in sorted(rel_dir.iterdir(), key=lambda p: p.name):
        if path.is_file():
            assets.append((path.name, path.stat().st_size, sha256_file(path)))

    log_range = f"{g['prev_tag']}..HEAD" if g["prev_tag"] else "HEAD~15..HEAD"
    rc_log, changelog = run_quiet(["git", "log", "--oneline", "-n", "20", log_range])
    if rc_log != 0 or not changelog:
        _, changelog = run_quiet(["git", "log", "--oneline", "-n", "15"])

    lines: list[str] = [
        f"## Atrium `{args.release_tag}` ({kind})",
        "",
        "| Field | Value |",
        "| :--- | :--- |",
        f"| Release Kind | **{kind}** |",
        f"| Build Number | `#{args.build_number}` |",
        f"| Profile | `{report_data.get('profile', 'release')}` |",
        f"| Architecture | `{report_data.get('arch', f'linux-{platform.machine()}')}` |",
        f"| Commit | `{g['short_sha']}` (`{g['full_sha']}`) |",
        "",
    ]

    steps = [StepRecord(**s) for s in report_data.get("steps", [])]
    tests = [TestSuiteMetric(**t) for t in report_data.get("tests", [])]
    bins = [BinaryArtifactMetric(**b) for b in report_data.get("binaries", [])]
    if steps or tests or bins:
        lines.append(
            format_build_summary_markdown(
                title="Build & Verification Report",
                steps=steps,
                tests=tests,
                binaries=bins,
                archive=None,
            )
        )

    lines.extend(
        [
            "### Release Assets & SHA-256 Checksums",
            "",
            "| File | Size | SHA-256 |",
            "| :--- | ---: | :--- |",
        ]
    )
    for name, sz, digest in assets:
        lines.append(f"| `{name}` | {human_size(sz)} | `{digest}` |")
    lines.append("")

    if changelog:
        lines.extend(
            [
                "### Recent Changes",
                "",
                "```text",
                changelog,
                "```",
                "",
            ]
        )

    lines.extend(
        [
            "### Quick Install & Systemd Service",
            "",
            "```bash",
            "tar -xzf atrium-*.tar.gz && cd atrium-*/",
            "sudo install -Dm755 bin/atriumd /usr/local/bin/atriumd",
            "sudo install -Dm755 bin/atrium /usr/local/bin/atrium",
            "sudo install -Dm755 bin/atrium-tui /usr/local/bin/atrium-tui",
            "sudo install -Dm755 bin/atrium-gui /usr/local/bin/atrium-gui",
            "",
            "# Option A: Manage via systemd system template service (root HID + per-user socket ACL)",
            "atrium daemon systemd install --bin /usr/local/bin/atriumd --enable --start",
            "",
            "# Option B: Or install udev rule + run user-session service or standalone `atrium daemon start`",
            "sudo cp udev/99-aula-win60he.rules /etc/udev/rules.d/ && sudo udevadm control --reload-rules",
            "atrium daemon systemd install --user --bin /usr/local/bin/atriumd --enable --start",
            "```",
            "",
        ]
    )

    body = "\n".join(lines)
    if args.output:
        Path(args.output).write_text(body, encoding="utf-8")
    append_text_file(args.step_summary, body)
    print(f"Generated release report ({len(assets)} assets).")
    return 0


def cmd_all(args: argparse.Namespace) -> int:
    rc = cmd_check(args)
    if rc != 0:
        return rc
    return cmd_build(args)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="atrium_build.py",
        description="Build, lint, test, package, and generate CI/release reports for Atrium.",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    p_meta = sub.add_parser("metadata", help="Resolve git/cargo metadata and release policy")
    p_meta.add_argument("--check-env-key", action="append", default=[], help="Env var required for release")
    p_meta.add_argument("--publish-release", default="auto", choices=("auto", "true", "false"))
    p_meta.add_argument("--prerelease", default="auto", choices=("auto", "true", "false"))
    p_meta.add_argument("--release-tag", default="")
    p_meta.add_argument("--github-output", default=None)
    p_meta.add_argument("--step-summary", default=None)
    p_meta.set_defaults(func=cmd_metadata)

    p_chk = sub.add_parser("check", help="Verify Rust toolchain and Linux native libraries")
    p_chk.add_argument("--step-summary", default=None)
    p_chk.set_defaults(func=cmd_check)

    def _add_build_flags(p: argparse.ArgumentParser) -> None:
        p.add_argument("--profile", default="release", choices=("release", "debug"))
        p.add_argument("--target", default=None, help="Optional Rust target triple")
        p.add_argument("--jobs", type=int, default=None, help="Parallel Cargo build jobs")
        p.add_argument("--locked", action="store_true", help="Pass --locked to Cargo commands")
        p.add_argument("--clean", action="store_true", help="Run cargo clean before building")
        p.add_argument("--fmt-check", action="store_true", help="Enforce cargo fmt --all -- --check")
        p.add_argument("--skip-clippy", action="store_true", help="Skip cargo clippy")
        p.add_argument("--skip-tests", action="store_true", help="Skip cargo test --workspace")
        p.add_argument("--no-strip", action="store_true", help="Do not strip release binaries")
        p.add_argument("--dist-dir", default="dist", help="Staging output directory")
        p.add_argument("--release-dir", default="release", help="Packaged archive output directory")
        p.add_argument("--release-tag", default=None, help="Version/tag label used in bundle name")
        p.add_argument("--step-summary", default=None)

    p_build = sub.add_parser("build", help="Run clippy, tests, compile binaries, and package tar.gz")
    _add_build_flags(p_build)
    p_build.set_defaults(func=cmd_build)

    p_all = sub.add_parser("all", help="Run environment check followed by full build & packaging")
    _add_build_flags(p_all)
    p_all.set_defaults(func=cmd_all)

    p_sum = sub.add_parser("checksums", help="Generate or refresh SHA256SUMS.txt in release directory")
    p_sum.add_argument("--release-dir", required=True)
    p_sum.set_defaults(func=cmd_checksums)

    p_rep = sub.add_parser("report", help="Generate release Markdown body and step summary")
    p_rep.add_argument("--release-dir", required=True)
    p_rep.add_argument("--report-json", default=None)
    p_rep.add_argument("--release-tag", required=True)
    p_rep.add_argument("--prerelease", default="true")
    p_rep.add_argument("--build-number", default="1")
    p_rep.add_argument("--output", default=None)
    p_rep.add_argument("--step-summary", default=None)
    p_rep.set_defaults(func=cmd_report)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
