//! hw-run – runs ffmpeg/ffprobe locked down. They parse files from untrusted downloads.
//!
//! ```text
//! hw-run [--read PATH]... [--write PATH]... [--nice N] -- PROGRAM [ARGS]...
//! hw-run --status
//! ```
//!
//! Before it executes PROGRAM (the process is replaced, no supervisor stays in between):
//!
//! * **Landlock** (no privileges or Docker options needed): PROGRAM may read only the `--read` paths and the
//!   system libraries, write only below `--write`, open no TCP connections, signal nothing outside its jail and
//!   reach no abstract unix sockets;
//! * **seccomp**: syscalls an audio converter never needs fail with EPERM – networking, ptrace, mounts,
//!   namespaces, kernel modules, BPF, keyrings, io_uring (which bypasses seccomp) …;
//! * `no_new_privs`, no core dumps, bounded memory and file size, lower CPU priority.
//!
//! The same command line as the launcher of Hörwerk 7.13 (`app/sandbox.py` there).
//! `FFMPEG_SANDBOX`: `auto` (jail when the kernel supports it), `require` (refuse without it), `off`.
//! `FFMPEG_MEMORY_MB` (default 4096), `FFMPEG_MAX_FILE_GB` (default 32).

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use landlock::{
    ABI, Access, AccessFs, AccessNet, BitFlags, CompatLevel, Compatible, LandlockStatus, PathBeneath, PathFd, Ruleset,
    RulesetAttr, RulesetCreatedAttr, RulesetStatus, Scope,
};
use seccompiler::{BpfProgram, SeccompAction, SeccompFilter, TargetArch};

/// Libraries and data the dynamic loader and ffmpeg need – read and execute, nothing else.
const SYSTEM_PATHS: &[&str] = &[
    "/lib",
    "/lib64",
    "/usr/lib",
    "/usr/lib64",
    "/usr/local/lib",
    "/etc/ld.so.cache",
    "/usr/share/zoneinfo",
    "/etc/localtime",
];
const ABI_WANTED: ABI = ABI::V6;

#[derive(Debug, Default, PartialEq)]
struct Args {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
    nice: i32,
    program: Vec<OsString>,
    status: bool,
}

fn parse(argv: impl IntoIterator<Item = OsString>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        match arg.to_str() {
            Some("--") => {
                out.program = it.collect();
                break;
            }
            Some("--status") => out.status = true,
            Some(flag @ ("--read" | "--write" | "--nice")) => {
                let value = it.next().ok_or_else(|| format!("{flag} braucht einen Wert"))?;
                match flag {
                    "--read" => out.read.push(PathBuf::from(value)),
                    "--write" => out.write.push(PathBuf::from(value)),
                    _ => {
                        out.nice = value
                            .to_str()
                            .and_then(|v| v.parse().ok())
                            .ok_or("--nice braucht eine Zahl")?
                    }
                }
            }
            _ => return Err(format!("unbekannte Option {}", arg.to_string_lossy())),
        }
    }
    if !out.status && out.program.is_empty() {
        return Err("kein Programm angegeben".into());
    }
    Ok(out)
}

fn which(program: &Path) -> PathBuf {
    if program.components().count() > 1 {
        return program.to_path_buf();
    }
    let path = env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into());
    env::split_paths(&path)
        .map(|d| d.join(program))
        .find(|p| p.is_file())
        .unwrap_or_else(|| program.to_path_buf())
}

fn env_number(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn limits(nice: i32) -> Result<(), String> {
    let set = |resource, value: u64| {
        let lim = libc::rlimit {
            rlim_cur: value,
            rlim_max: value,
        };
        // SAFETY: plain syscall with a valid struct
        if unsafe { libc::setrlimit(resource, &lim) } != 0 {
            return Err(format!("setrlimit: {}", std::io::Error::last_os_error()));
        }
        Ok(())
    };
    set(libc::RLIMIT_CORE, 0)?;
    let memory = env_number("FFMPEG_MEMORY_MB", 4096);
    if memory > 0 {
        set(libc::RLIMIT_AS, memory << 20)?;
    }
    let file = env_number("FFMPEG_MAX_FILE_GB", 32);
    if file > 0 {
        set(libc::RLIMIT_FSIZE, file << 30)?;
    }
    if nice != 0 {
        // SAFETY: plain syscall; a failure (already lower) is harmless
        unsafe { libc::nice(nice) };
    }
    Ok(())
}

/// The Landlock ABI of the running kernel (0: none) and, without one, why: "kernel" (not built in), "disabled" (built in but not switched on
/// at boot) or "blocked" (the container's seccomp profile forbids the call).
fn landlock_probe() -> (i32, &'static str) {
    // SAFETY: LANDLOCK_CREATE_RULESET_VERSION query with null attributes, as documented
    let v = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, std::ptr::null::<u8>(), 0usize, 1u32) };
    if v > 0 {
        return (v as i32, "");
    }
    let reason = match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::EOPNOTSUPP) => "disabled",
        Some(libc::EPERM) | Some(libc::EACCES) => "blocked",
        _ => "kernel",
    };
    (0, reason)
}

fn rules(paths: &[PathBuf], access: BitFlags<AccessFs>) -> Vec<PathBeneath<PathFd>> {
    paths
        .iter()
        .filter_map(|p| PathFd::new(p).ok().map(|fd| (p, fd))) // a missing path (e.g. /lib64 on arm64) is no error
        .map(|(p, fd)| {
            // a rule on a file may only grant file rights
            let rights = if p.is_dir() {
                access
            } else {
                access & AccessFs::from_file(ABI_WANTED)
            };
            PathBeneath::new(fd, rights)
        })
        .collect()
}

fn landlock(read: &[PathBuf], write: &[PathBuf], execute: &[PathBuf]) -> Result<bool, String> {
    let all = AccessFs::from_all(ABI_WANTED);
    let ro = AccessFs::from_read(ABI_WANTED);
    let status = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(all)
        .and_then(|r| r.handle_access(AccessNet::BindTcp | AccessNet::ConnectTcp))
        .and_then(|r| r.scope(Scope::AbstractUnixSocket | Scope::Signal))
        .and_then(|r| r.create())
        .and_then(|r| {
            r.add_rules(
                rules(execute, ro | AccessFs::Execute)
                    .into_iter()
                    .map(Ok::<_, landlock::RulesetError>),
            )
        })
        .and_then(|r| r.add_rules(rules(read, ro).into_iter().map(Ok::<_, landlock::RulesetError>)))
        .and_then(|r| r.add_rules(rules(write, all).into_iter().map(Ok::<_, landlock::RulesetError>)))
        .and_then(|r| {
            r.add_rules(
                rules(&[PathBuf::from("/dev/null")], AccessFs::ReadFile | AccessFs::WriteFile)
                    .into_iter()
                    .chain(rules(&[PathBuf::from("/dev/urandom")], AccessFs::ReadFile.into()))
                    .map(Ok::<_, landlock::RulesetError>),
            )
        })
        .and_then(|r| r.restrict_self())
        .map_err(|e| format!("Landlock: {e}"))?;
    Ok(!matches!(
        status.landlock,
        LandlockStatus::NotEnabled | LandlockStatus::NotImplemented
    ) && status.ruleset != RulesetStatus::NotEnforced)
}

/// Syscalls an audio converter never needs – they fail with EPERM instead of reaching the kernel.
fn denied_syscalls() -> Vec<i64> {
    vec![
        libc::SYS_socket,
        libc::SYS_socketpair,
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_accept,
        libc::SYS_accept4,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_kexec_load,
        libc::SYS_reboot,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_userfaultfd,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        libc::SYS_open_by_handle_at,
        libc::SYS_name_to_handle_at,
        libc::SYS_fsopen,
        libc::SYS_fsmount,
        libc::SYS_move_mount,
        libc::SYS_open_tree,
    ]
}

fn seccomp() -> Result<(), String> {
    let arch = match env::consts::ARCH {
        "x86_64" => TargetArch::x86_64,
        "aarch64" => TargetArch::aarch64,
        other => return Err(format!("seccomp: unbekannte Architektur {other}")),
    };
    let rules: BTreeMap<i64, Vec<seccompiler::SeccompRule>> =
        denied_syscalls().into_iter().map(|s| (s, vec![])).collect();
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        arch,
    )
    .map_err(|e| format!("seccomp: {e}"))?;
    let program: BpfProgram = filter.try_into().map_err(|e| format!("seccomp: {e}"))?;
    seccompiler::apply_filter(&program).map_err(|e| format!("seccomp: {e}"))
}

fn no_new_privs() -> Result<(), String> {
    // SAFETY: PR_SET_NO_NEW_PRIVS with the documented arguments
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(format!("no_new_privs: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

fn run(args: Args) -> Result<(), (u8, String)> {
    let mode = env::var("FFMPEG_SANDBOX")
        .unwrap_or_else(|_| "auto".into())
        .trim()
        .to_lowercase();
    if args.status {
        let (abi, reason) = landlock_probe();
        println!(
            "{{\"mode\":\"{mode}\",\"landlock_abi\":{abi},\"reason\":\"{reason}\",\"active\":{},\"network_blocked\":{},\"seccomp\":{}}}",
            mode != "off" && abi > 0,
            mode != "off" && abi >= 4,
            mode != "off"
        );
        return Ok(());
    }
    let program = which(Path::new(&args.program[0]));
    let program = program.canonicalize().unwrap_or(program);
    limits(args.nice).map_err(|e| (126, e))?;
    if mode != "off" {
        no_new_privs().map_err(|e| (126, e))?;
        let mut execute: Vec<PathBuf> = vec![program.clone()];
        execute.extend(SYSTEM_PATHS.iter().map(PathBuf::from));
        let jailed = match landlock(&args.read, &args.write, &execute) {
            Ok(on) => on,
            Err(e) if mode == "require" => return Err((126, e)),
            Err(_) => false,
        };
        if !jailed && mode == "require" {
            return Err((126, "Landlock nicht verfügbar (FFMPEG_SANDBOX=require)".into()));
        }
        // last: after this, even hw-run itself can no longer open sockets
        seccomp().map_err(|e| (126, e))?;
    }
    let err = Command::new(&program)
        .arg0(&args.program[0])
        .args(&args.program[1..])
        .exec();
    Err((127, format!("{}: {err}", program.display())))
}

fn main() -> ExitCode {
    let args = match parse(env::args_os().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("hw-run: {e}");
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, msg)) => {
            eprintln!("hw-run: {msg}");
            ExitCode::from(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_the_launcher_command_line() {
        let a = parse(os(&[
            "--read",
            "/in/a.mp3",
            "--write",
            "/work",
            "--nice",
            "10",
            "--",
            "ffmpeg",
            "-i",
            "x",
        ]))
        .unwrap();
        assert_eq!(a.read, vec![PathBuf::from("/in/a.mp3")]);
        assert_eq!(a.write, vec![PathBuf::from("/work")]);
        assert_eq!(a.nice, 10);
        assert_eq!(a.program, os(&["ffmpeg", "-i", "x"]));
    }

    #[test]
    fn refuses_unknown_or_incomplete_options() {
        assert!(parse(os(&["--evil", "x", "--", "ffmpeg"])).is_err());
        assert!(parse(os(&["--read"])).is_err());
        assert!(parse(os(&["--nice", "laut", "--", "ffmpeg"])).is_err());
        assert!(parse(os(&["--read", "/x"])).is_err()); // no program
        assert!(parse(os(&["--status"])).unwrap().status);
    }
}
