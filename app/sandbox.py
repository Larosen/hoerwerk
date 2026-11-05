"""Runs ffmpeg/ffprobe locked down – they parse files from untrusted downloads.

Every call goes through this file as a tiny launcher (``python -I sandbox.py … -- ffmpeg …``)
which, before it executes ffmpeg:

* applies a **Landlock** filesystem jail (Linux ≥ 5.13, no privileges or Docker options
  needed): ffmpeg may read only the files of this one call, write only into its work
  folder, and – with Landlock ABI 4+ – open no TCP connections. ``/config`` (database
  with API keys), the rest of the library and the system stay out of reach;
* sets ``no_new_privs`` and resource limits (no core dumps, bounded memory and file size);
* lowers the CPU priority (``NICE``).

The ffmpeg built for the Docker image has no network code at all and only the audio
formats Hörwerk reads (``docker/build-ffmpeg.sh``) – the jail is the second wall.

``FFMPEG_SANDBOX``: ``auto`` (default: jail when the kernel supports it), ``require``
(refuse to run ffmpeg without it), ``off``.

**hw-run** (Rust, ``rust/hw-run``, in the image at ``/usr/local/bin/hw-run``) is the same launcher with the same
command line, plus a seccomp filter (no sockets, ptrace, mounts, namespaces, BPF, io_uring …) and Landlock signal and
abstract-socket scoping. When it is there it is used; this file's launcher is the fallback (``HW_RUN`` overrides).
"""
from __future__ import annotations

import ctypes
import os
import platform
import struct
import sys
from typing import Iterable, List, Sequence

FFMPEG = os.environ.get("FFMPEG", "ffmpeg")
FFPROBE = os.environ.get("FFPROBE", "ffprobe")
MODE = os.environ.get("FFMPEG_SANDBOX", "auto").strip().lower()
MEMORY_MB = int(os.environ.get("FFMPEG_MEMORY_MB", "4096"))
MAX_FILE_GB = int(os.environ.get("FFMPEG_MAX_FILE_GB", "32"))
HERE = os.path.abspath(__file__)


def hw_run() -> str:
    """Path of the Rust launcher, "" when it is not installed (HW_RUN=off forces the Python launcher)."""
    want = os.environ.get("HW_RUN", "")
    if want.lower() == "off":
        return ""
    if want:
        return want if os.access(want, os.X_OK) else ""
    for d in ("/usr/local/bin", *os.environ.get("PATH", "").split(os.pathsep)):
        p = os.path.join(d, "hw-run")
        if d and os.path.isfile(p) and os.access(p, os.X_OK):
            return p
    return ""

# --- Landlock (include/uapi/linux/landlock.h) --------------------------------
SYS_CREATE, SYS_ADD_RULE, SYS_RESTRICT = 444, 445, 446      # same numbers on x86_64 and arm64
CREATE_RULESET_VERSION = 1
RULE_PATH_BENEATH = 1
EXECUTE, WRITE_FILE, READ_FILE, READ_DIR = 1 << 0, 1 << 1, 1 << 2, 1 << 3
REMOVE_DIR, REMOVE_FILE, MAKE_CHAR, MAKE_DIR, MAKE_REG = 1 << 4, 1 << 5, 1 << 6, 1 << 7, 1 << 8
MAKE_SOCK, MAKE_FIFO, MAKE_BLOCK, MAKE_SYM = 1 << 9, 1 << 10, 1 << 11, 1 << 12
REFER, TRUNCATE, IOCTL_DEV = 1 << 13, 1 << 14, 1 << 15
NET_BIND_TCP, NET_CONNECT_TCP = 1 << 0, 1 << 1
SCOPE_ABSTRACT_UNIX_SOCKET, SCOPE_SIGNAL = 1 << 0, 1 << 1
FILE_RIGHTS = EXECUTE | WRITE_FILE | READ_FILE | TRUNCATE | IOCTL_DEV      # rights a rule on a file may grant
SYSTEM_PATHS = ("/lib", "/lib64", "/usr/lib", "/usr/lib64", "/usr/local/lib", "/etc/ld.so.cache",
                "/usr/share/zoneinfo", "/etc/localtime")


class _RulesetAttr(ctypes.Structure):
    _fields_ = [("handled_access_fs", ctypes.c_uint64), ("handled_access_net", ctypes.c_uint64),
                ("scoped", ctypes.c_uint64)]


def _path_beneath(allowed: int, fd: int) -> ctypes.Array:
    """struct landlock_path_beneath_attr {u64 allowed_access; s32 parent_fd;} __attribute__((packed))"""
    return ctypes.create_string_buffer(struct.pack("=Qi", allowed, fd), 12)


def _libc():
    return ctypes.CDLL(None, use_errno=True)


def landlock_abi() -> int:
    """Landlock ABI version of the running kernel, 0 when unavailable (or blocked by seccomp)."""
    if platform.system() != "Linux" or platform.machine() not in ("x86_64", "aarch64", "arm64"):
        return 0
    try:
        v = _libc().syscall(SYS_CREATE, None, ctypes.c_size_t(0), ctypes.c_uint32(CREATE_RULESET_VERSION))
    except (OSError, AttributeError):
        return 0
    return v if v > 0 else 0


def _handled_fs(abi: int) -> int:
    rights = (EXECUTE | WRITE_FILE | READ_FILE | READ_DIR | REMOVE_DIR | REMOVE_FILE | MAKE_CHAR | MAKE_DIR
              | MAKE_REG | MAKE_SOCK | MAKE_FIFO | MAKE_BLOCK | MAKE_SYM)
    if abi >= 2:
        rights |= REFER
    if abi >= 3:
        rights |= TRUNCATE
    if abi >= 5:
        rights |= IOCTL_DEV
    return rights


def apply_landlock(read: Iterable[str], write: Iterable[str], execute: Iterable[str]) -> int:
    """Restrict this process (and what it executes) to the given paths. Returns the ABI used."""
    abi = landlock_abi()
    if not abi:
        return 0
    libc = _libc()
    handled = _handled_fs(abi)
    attr = _RulesetAttr(handled, NET_BIND_TCP | NET_CONNECT_TCP if abi >= 4 else 0,
                        SCOPE_ABSTRACT_UNIX_SOCKET | SCOPE_SIGNAL if abi >= 6 else 0)
    size = 24 if abi >= 6 else 16 if abi >= 4 else 8
    fd = libc.syscall(SYS_CREATE, ctypes.byref(attr), ctypes.c_size_t(size), ctypes.c_uint32(0))
    if fd < 0:
        raise OSError(ctypes.get_errno(), "landlock_create_ruleset")
    ro = READ_FILE | READ_DIR
    rw = ro | WRITE_FILE | MAKE_REG | MAKE_DIR | REMOVE_FILE | REMOVE_DIR | (TRUNCATE if abi >= 3 else 0)
    rules = [(p, ro | EXECUTE) for p in execute] + [(p, ro) for p in read] + [(p, rw) for p in write]
    rules += [("/dev/null", READ_FILE | WRITE_FILE), ("/dev/urandom", READ_FILE)]
    try:
        for path, rights in rules:
            try:
                pfd = os.open(path, os.O_PATH | os.O_CLOEXEC)
            except OSError:
                continue                       # e.g. /lib64 does not exist on arm64
            try:
                if not os.path.isdir(path):
                    rights &= FILE_RIGHTS
                rule = _path_beneath(rights & handled, pfd)
                if libc.syscall(SYS_ADD_RULE, fd, RULE_PATH_BENEATH, rule, ctypes.c_uint32(0)) != 0:
                    raise OSError(ctypes.get_errno(), f"landlock_add_rule {path}")
            finally:
                os.close(pfd)
        if libc.prctl(38, 1, 0, 0, 0) != 0:      # PR_SET_NO_NEW_PRIVS
            raise OSError(ctypes.get_errno(), "no_new_privs")
        if libc.syscall(SYS_RESTRICT, fd, ctypes.c_uint32(0)) != 0:
            raise OSError(ctypes.get_errno(), "landlock_restrict_self")
    finally:
        os.close(fd)
    return abi


# --------------------------------------------------------------------------- #
# Command lines for the app
# --------------------------------------------------------------------------- #

def command(binary: str, args: Sequence[str], read: Iterable[str] = (), write: Iterable[str] = (),
            nice: str = "") -> List[str]:
    """argv that runs ``binary args`` jailed to ``read``/``write``."""
    if MODE == "off":
        cmd = [binary, *args]
        return (["nice", "-n", nice] + cmd) if nice and nice != "0" else cmd
    rust = hw_run()
    launcher = [rust] if rust else [sys.executable, "-I", "-S", HERE]
    for p in read:
        launcher += ["--read", os.path.abspath(p)]
    for p in write:
        launcher += ["--write", os.path.abspath(p)]
    if nice and nice != "0":
        launcher += ["--nice", str(nice)]
    return launcher + ["--", binary, *args]


def ffmpeg(args: Sequence[str], read: Iterable[str] = (), write: Iterable[str] = (), nice: str = "") -> List[str]:
    return command(FFMPEG, args, read, write, nice)


def ffprobe(args: Sequence[str], read: Iterable[str] = ()) -> List[str]:
    return command(FFPROBE, args, read)


def status() -> dict:
    abi = landlock_abi()
    rust = hw_run()
    return {"mode": MODE, "landlock_abi": abi, "active": MODE != "off" and abi > 0,
            "network_blocked": MODE != "off" and (abi >= 4 or bool(rust)),
            "launcher": "hw-run" if rust else "python", "seccomp": MODE != "off" and bool(rust)}


# --------------------------------------------------------------------------- #
# Launcher
# --------------------------------------------------------------------------- #

def _which(binary: str) -> str:
    if os.sep in binary:
        return binary
    for d in os.environ.get("PATH", "/usr/bin:/bin").split(os.pathsep):
        p = os.path.join(d, binary)
        if os.path.isfile(p) and os.access(p, os.X_OK):
            return p
    return binary


def main(argv: List[str]) -> int:
    import resource

    read: List[str] = []
    write: List[str] = []
    nice = 0
    i = 0
    while i < len(argv) and argv[i] != "--":
        flag, value = argv[i], argv[i + 1]
        if flag == "--read":
            read.append(value)
        elif flag == "--write":
            write.append(value)
        elif flag == "--nice":
            nice = int(value)
        else:
            print(f"sandbox: unbekannte Option {flag}", file=sys.stderr)
            return 2
        i += 2
    cmd = argv[i + 1:]
    if not cmd:
        print("sandbox: kein Programm angegeben", file=sys.stderr)
        return 2
    binary = os.path.realpath(_which(cmd[0]))

    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    if MEMORY_MB > 0:
        resource.setrlimit(resource.RLIMIT_AS, (MEMORY_MB << 20, MEMORY_MB << 20))
    if MAX_FILE_GB > 0:
        resource.setrlimit(resource.RLIMIT_FSIZE, (MAX_FILE_GB << 30, MAX_FILE_GB << 30))
    if nice:
        try:
            os.nice(nice)
        except OSError:
            pass
    try:
        abi = apply_landlock(read, write, [binary, *SYSTEM_PATHS])
    except OSError as e:
        abi = 0
        if MODE == "require":
            print(f"sandbox: Landlock fehlgeschlagen: {e}", file=sys.stderr)
            return 126
    if not abi and MODE == "require":
        print("sandbox: Landlock nicht verfügbar (FFMPEG_SANDBOX=require)", file=sys.stderr)
        return 126
    os.execv(binary, [cmd[0], *cmd[1:]])
    return 127


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
