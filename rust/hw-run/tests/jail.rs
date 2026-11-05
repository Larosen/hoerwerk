//! The jail from the outside: what a program run by hw-run may and may not do.
//! Needs a kernel with Landlock (ABI ≥ 4); otherwise the tests that depend on it are skipped.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn hw_run() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hw-run"));
    c.env("FFMPEG_SANDBOX", "auto");
    c
}

fn landlock_abi() -> i64 {
    let out = hw_run().arg("--status").output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    text.split("\"landlock_abi\":")
        .nth(1)
        .and_then(|r| r.split(',').next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn tempdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hw-run-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn sh(read: &[&Path], write: &[&Path], script: &str) -> Output {
    let mut c = hw_run();
    for r in read {
        c.arg("--read").arg(r);
    }
    for w in write {
        c.arg("--write").arg(w);
    }
    c.args(["--", "/bin/sh", "-c", script]).output().unwrap()
}

#[test]
fn reads_only_what_it_was_given() {
    if landlock_abi() < 1 {
        return;
    }
    let d = tempdir("read");
    let ok = d.join("ok.txt");
    let secret = d.join("secret.txt");
    fs::write(&ok, "hallo\n").unwrap();
    fs::write(&secret, "api-key\n").unwrap();
    let out = sh(&[&ok], &[], &format!("read x < {} && echo $x", ok.display()));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hallo", "{out:?}");
    let out = sh(&[&ok], &[], &format!("read x < {} && echo $x", secret.display()));
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
    fs::remove_dir_all(d).unwrap();
}

#[test]
fn writes_only_below_its_work_folder() {
    if landlock_abi() < 1 {
        return;
    }
    let d = tempdir("write");
    let work = d.join("work");
    fs::create_dir_all(&work).unwrap();
    let out = sh(&[], &[&work], &format!("echo ok > {}/out.m4b", work.display()));
    assert!(out.status.success(), "{out:?}");
    assert!(work.join("out.m4b").exists());
    let out = sh(&[], &[&work], &format!("echo bad > {}/escape.txt", d.display()));
    assert!(!out.status.success());
    assert!(!d.join("escape.txt").exists());
    fs::remove_dir_all(d).unwrap();
}

#[test]
fn no_network() {
    // seccomp: no socket at all (independent of Landlock); a shell's /dev/tcp needs one
    if !Path::new("/usr/bin/python3").exists() {
        return;
    }
    let out = Command::new(env!("CARGO_BIN_EXE_hw-run"))
        .args(["--", "/usr/bin/python3", "-I", "-c", "import socket; socket.socket()"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not permitted"),
        "{out:?}"
    );
}

#[test]
fn cannot_signal_outside_its_jail() {
    if landlock_abi() < 6 {
        return;
    }
    let mut victim = Command::new("sleep").arg("30").spawn().unwrap();
    let out = sh(&[], &[], &format!("kill -9 {}", victim.id()));
    assert!(!out.status.success(), "{out:?}");
    assert!(victim.try_wait().unwrap().is_none(), "the process outside was killed");
    victim.kill().unwrap();
    victim.wait().unwrap();
}

#[test]
fn require_refuses_without_a_jail_only_when_there_is_none() {
    let out = hw_run()
        .env("FFMPEG_SANDBOX", "require")
        .args(["--", "/bin/true"])
        .output()
        .unwrap();
    assert_eq!(out.status.success(), landlock_abi() >= 1, "{out:?}");
}

#[test]
fn off_runs_unrestricted() {
    let d = tempdir("off");
    let out = hw_run()
        .env("FFMPEG_SANDBOX", "off")
        .args(["--", "/bin/sh", "-c"])
        .arg(format!("echo free > {}/x", d.display()))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    fs::remove_dir_all(d).unwrap();
}

#[test]
fn exit_code_and_output_of_the_program_pass_through() {
    let out = sh(&[], &[], "echo out; echo err >&2; exit 7");
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "out\n");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "err\n");
}

#[test]
fn ffmpeg_works_inside() {
    let Some(ffmpeg) = ["/usr/bin/ffmpeg", "/usr/local/bin/ffmpeg"]
        .into_iter()
        .find(|p| Path::new(p).exists())
    else {
        return;
    };
    let d = tempdir("ffmpeg");
    let out = hw_run()
        .args(["--write"])
        .arg(&d)
        .args([
            "--",
            ffmpeg,
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=d=0.5",
            "-c:a",
            "aac",
        ])
        .arg(d.join("x.m4a"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(d.join("x.m4a").metadata().unwrap().len() > 100);
    fs::remove_dir_all(d).unwrap();
}
