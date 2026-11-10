//! `launchpad netcap --port <capture port> --key <proxy key> -- <program> …`: the network capture of contained
//! sessions (policy and design: `launchpad_core::netcap`).
//!
//! The supervisor starts the program under a seccomp filter that no process of the session can remove
//! (`no_new_privs`), and answers its network system calls through a user notification listener:
//! - It creates every internet socket itself and injects it into the agent (`SECCOMP_IOCTL_NOTIF_ADDFD`), so it
//!   knows each socket's kind and keeps a copy of TCP sockets until they connect (`pidfd_getfd` is refused in
//!   HF Jobs).
//! - A TCP connect to a remote address connects that copy to the runner proxy's capture port instead, with a
//!   header naming the destination. The proxy connects to the destination first, so `connect` returns only once
//!   the destination is reached, with its real error otherwise (fallbacks between addresses keep working).
//! - Loopback connects go to their address. UDP reaches port 53 only. Link-local and metadata addresses are refused
//!   (`policy::refused`). Other socket families, io_uring and TCP Fast Open are refused.
//! - Unix sockets reach paths, never the abstract namespace (the machine's X11 and D-Bus share it with the jail):
//!   the supervisor runs `connect` and `bind` of a Unix socket itself, on a copy of the descriptor, with the
//!   address it checked, so the agent cannot change the address after the check. Addresses given to `sendto` and
//!   `sendmsg` are checked in place.
//!
//! It reads the agent's memory and descriptors (`process_vm_readv`, `/proc/<pid>/fd`, `pidfd_getfd`), which the
//! kernel allows like a ptrace attach:
//! - Yama allows ancestors only: the supervisor is the agent's parent and the subreaper of the processes that
//!   detach (Chrome's crash handler double-forks).
//! - Non-dumpable processes (Chrome's helpers, ssh-agent) need `CAP_SYS_PTRACE` in their user namespace: the
//!   program starts in a new user namespace that the supervisor owns and holds every capability over, while the
//!   program drops all of its own (`Reach::Namespace`). Where user namespaces are refused (HF Jobs), the filter
//!   makes `prctl(PR_SET_DUMPABLE, 0)` succeed without effect instead (`Reach::KeepDumpable`).
//! - A process it still cannot read gets EFAULT for the checked calls: never an unchecked network call.
//!
//! It forwards the signals sent to it and exits with the program's status. Processes still running after the
//! program exits lose their network calls (the listener closes with the supervisor).

use std::collections::{HashMap, VecDeque};
use std::ffi::CString;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use launchpad_core::netcap as policy;

// ------------------------------------------------------------------ the filter

#[repr(C)]
#[derive(Clone, Copy)]
struct SockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *const SockFilter,
}

const LD_W_ABS: u16 = 0x20;
const JEQ_K: u16 = 0x15;
const JGE_K: u16 = 0x35;
const JSET_K: u16 = 0x45;
const RET_K: u16 = 0x06;

#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH: u32 = 0xc000_003e;
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH: u32 = 0xc000_00b7;

const RET_ALLOW: u32 = libc::SECCOMP_RET_ALLOW;
const RET_NOTIF: u32 = libc::SECCOMP_RET_USER_NOTIF;
const fn ret_errno(e: i32) -> u32 {
    libc::SECCOMP_RET_ERRNO | (e as u32 & 0xffff)
}

/// Offsets in `struct seccomp_data` (little-endian: the low word of an argument comes first).
const NR: u32 = 0;
const ARCH: u32 = 4;
const fn arg_lo(i: u32) -> u32 {
    16 + 8 * i
}
const fn arg_hi(i: u32) -> u32 {
    20 + 8 * i
}

#[derive(Clone, Copy)]
enum To {
    Next,
    L(&'static str),
}

enum Ins {
    Label(&'static str),
    Ld(u32),
    Jeq(u32, To, To),
    Jge(u32, To, To),
    Jset(u32, To, To),
    Ret(u32),
}

/// Assembles instructions with labels (forward jumps only).
fn assemble(prog: &[Ins]) -> Vec<SockFilter> {
    let mut at = HashMap::new();
    let mut n = 0usize;
    for i in prog {
        match i {
            Ins::Label(l) => {
                at.insert(*l, n);
            }
            _ => n += 1,
        }
    }
    let mut out = vec![];
    for i in prog {
        let here = out.len();
        let off = |t: To| -> u8 {
            match t {
                To::Next => 0,
                To::L(l) => u8::try_from(at[l] - here - 1).expect("short jump"),
            }
        };
        out.push(match *i {
            Ins::Label(_) => continue,
            Ins::Ld(k) => SockFilter { code: LD_W_ABS, jt: 0, jf: 0, k },
            Ins::Jeq(k, t, f) => SockFilter { code: JEQ_K, jt: off(t), jf: off(f), k },
            Ins::Jge(k, t, f) => SockFilter { code: JGE_K, jt: off(t), jf: off(f), k },
            Ins::Jset(k, t, f) => SockFilter { code: JSET_K, jt: off(t), jf: off(f), k },
            Ins::Ret(k) => SockFilter { code: RET_K, jt: 0, jf: 0, k },
        });
    }
    out
}

/// The filter. `handoff`: the one descriptor allowed to `sendmsg` unchecked, which carries the listener to the
/// supervisor before the program starts. `keep_dumpable`: `prctl(PR_SET_DUMPABLE, 0)` succeeds without effect,
/// for machines where the supervisor cannot read non-dumpable processes (see `Reach`).
fn filter(handoff: RawFd, keep_dumpable: bool) -> Vec<SockFilter> {
    use Ins::*;
    use To::*;
    let nr = |n: libc::c_long| n as u32;
    let mut p = vec![Ld(ARCH), Jeq(AUDIT_ARCH, Next, L("enosys")), Ld(NR)];
    // x32 system calls on x86_64.
    #[cfg(target_arch = "x86_64")]
    p.push(Jge(0x4000_0000, L("enosys"), Next));
    #[cfg(not(target_arch = "x86_64"))]
    let _ = Jge(0, Next, Next);
    p.extend([
        Jeq(nr(libc::SYS_socket), L("socket"), Next),
        Jeq(nr(libc::SYS_socketpair), L("socketpair"), Next),
        Jeq(nr(libc::SYS_connect), L("notif"), Next),
        Jeq(nr(libc::SYS_bind), L("notif"), Next),
        Jeq(nr(libc::SYS_sendto), L("sendto"), Next),
        Jeq(nr(libc::SYS_sendmsg), L("sendmsg"), Next),
        Jeq(nr(libc::SYS_sendmmsg), L("sendmmsg"), Next),
        Jeq(nr(libc::SYS_getpeername), L("notif"), Next),
        Jeq(nr(libc::SYS_listen), L("notif"), Next),
        Jeq(nr(libc::SYS_io_uring_setup), L("enosys"), Next),
        Jeq(nr(libc::SYS_io_uring_enter), L("enosys"), Next),
        Jeq(nr(libc::SYS_io_uring_register), L("enosys"), Next),
        Jeq(nr(libc::SYS_prctl), L("prctl"), Next),
        Ret(RET_ALLOW),
        Label("prctl"),
        Ld(arg_lo(0)),
        Jeq(libc::PR_SET_DUMPABLE as u32, Next, L("allow")),
        Ld(arg_lo(1)),
        Jeq(0, if keep_dumpable { L("success") } else { L("allow") }, L("allow")),
        Label("socket"),
        Ld(arg_lo(0)),
        Jeq(libc::AF_UNIX as u32, L("allow"), Next),
        Jeq(libc::AF_INET as u32, L("notif"), Next),
        Jeq(libc::AF_INET6 as u32, L("notif"), Next),
        Jeq(libc::AF_NETLINK as u32, Next, L("eafnosupport")),
        Ld(arg_lo(2)),
        Jeq(libc::NETLINK_ROUTE as u32, L("allow"), L("eprotonosupport")),
        Label("socketpair"),
        Ld(arg_lo(0)),
        Jeq(libc::AF_UNIX as u32, L("allow"), L("eafnosupport")),
        Label("sendto"),
        Ld(arg_lo(3)),
        Jset(libc::MSG_FASTOPEN as u32, L("eopnotsupp"), Next),
        Ld(arg_lo(4)),
        Jeq(0, Next, L("notif")),
        Ld(arg_hi(4)),
        Jeq(0, L("allow"), L("notif")),
        Label("sendmsg"),
        Ld(arg_lo(0)),
        Jeq(handoff as u32, L("allow"), Next),
        Ld(arg_lo(2)),
        Jset(libc::MSG_FASTOPEN as u32, L("eopnotsupp"), L("notif")),
        Label("sendmmsg"),
        Ld(arg_lo(3)),
        Jset(libc::MSG_FASTOPEN as u32, L("eopnotsupp"), L("notif")),
        Label("notif"),
        Ret(RET_NOTIF),
        Label("allow"),
        Ret(RET_ALLOW),
        // Returns 0 without running the system call.
        Label("success"),
        Ret(ret_errno(0)),
        Label("enosys"),
        Ret(ret_errno(libc::ENOSYS)),
        Label("eafnosupport"),
        Ret(ret_errno(libc::EAFNOSUPPORT)),
        Label("eprotonosupport"),
        Ret(ret_errno(libc::EPROTONOSUPPORT)),
        Label("eopnotsupp"),
        Ret(ret_errno(libc::EOPNOTSUPP)),
    ]);
    assemble(&p)
}

// ------------------------------------------------------------------ start

static CHILD: AtomicI32 = AtomicI32::new(0);

/// Signals sent to the supervisor with `kill` go to the program too. Signals from the terminal (Ctrl+C) already
/// reach the program's process group: they are not sent twice.
extern "C" fn forward(sig: libc::c_int, info: *mut libc::siginfo_t, _: *mut libc::c_void) {
    // SAFETY: the kernel passes a valid siginfo to an SA_SIGINFO handler; kill is async-signal-safe.
    unsafe {
        if !info.is_null() && (*info).si_code <= 0 {
            let child = CHILD.load(Ordering::Relaxed);
            if child > 0 {
                libc::kill(child, sig);
            }
        }
    }
}

const FORWARDED: &[libc::c_int] = &[libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT, libc::SIGUSR1, libc::SIGUSR2];

fn fail(text: &str) -> ! {
    eprintln!("launchpad netcap: {text}");
    std::process::exit(126);
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO)
}

/// `launchpad netcap …`: never returns.
pub fn main(args: &[String]) -> ! {
    let mut port = None;
    let mut key = None;
    let mut it = args.iter();
    let mut program = vec![];
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => port = it.next().and_then(|p| p.parse::<u16>().ok()),
            "--key" => key = it.next().cloned(),
            "--self-test" => self_test(),
            "--" => {
                program = it.by_ref().cloned().collect();
                break;
            }
            other => fail(&format!("unknown argument {other}")),
        }
    }
    let (Some(port), Some(key)) = (port, key) else { fail("usage: launchpad netcap --port <port> --key <key> -- <program> [args…]") };
    if program.is_empty() {
        fail("no program to run");
    }
    run(port, key, &program)
}

fn run(port: u16, key: String, program: &[String]) -> ! {
    // Everything the child needs is prepared before fork.
    let argv: Vec<CString> = program.iter().map(|a| CString::new(a.as_bytes()).unwrap_or_else(|_| fail("argument with a NUL byte"))).collect();
    let mut argv_ptrs: Vec<*const libc::c_char> = argv.iter().map(|a| a.as_ptr()).collect();
    argv_ptrs.push(std::ptr::null());
    let has_slash = program[0].contains('/');
    let mut pair = [0; 2];
    // SAFETY: plain system calls on descriptors this process owns.
    unsafe {
        if libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0, pair.as_mut_ptr()) != 0 {
            fail("socketpair");
        }
    }
    // The child's end, on a high number: the filter lets only this descriptor `sendmsg` unchecked.
    // SAFETY: as above.
    let handoff = unsafe { libc::fcntl(pair[1], libc::F_DUPFD_CLOEXEC, 700) };
    if handoff < 0 {
        fail("dup");
    }
    // SAFETY: as above.
    unsafe { libc::close(pair[1]) };
    let reach = Reach::detect();
    let (prog, keep_prog) = (filter(handoff, false), filter(handoff, true));
    let fprog = SockFprog { len: prog.len() as u16, filter: prog.as_ptr() };
    let keep_fprog = SockFprog { len: keep_prog.len() as u16, filter: keep_prog.as_ptr() };
    // SAFETY: plain system calls.
    let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
    let uid_map = format!("{uid} {uid} 1");
    let gid_map = format!("{gid} {gid} 1");
    let mut ack = [0u8; 1];
    // SAFETY: fork in a single-threaded process (netcap runs before any runtime starts). The child only makes
    // system calls on memory prepared above, then execs.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        fail("fork");
    }
    if pid == 0 {
        // SAFETY: see above; every pointer refers to memory prepared before fork.
        unsafe {
            libc::close(pair[0]);
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            let keep_dumpable = match reach {
                Reach::Capable => false,
                Reach::KeepDumpable => true,
                Reach::Namespace => !own_namespace(handoff, uid_map.as_bytes(), gid_map.as_bytes()),
            };
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                child_fail(handoff, b"no_new_privs refused\n");
            }
            let fprog: *const SockFprog = if keep_dumpable { &keep_fprog } else { &fprog };
            let listener = libc::syscall(libc::SYS_seccomp, libc::SECCOMP_SET_MODE_FILTER, libc::SECCOMP_FILTER_FLAG_NEW_LISTENER, fprog) as libc::c_int;
            if listener < 0 {
                child_fail(handoff, b"seccomp user notification refused (Linux 5.9 or newer is needed)\n");
            }
            send_fd(handoff, listener);
            // Wait until the supervisor holds the listener.
            libc::read(handoff, ack.as_mut_ptr().cast(), 1);
            libc::close(listener);
            libc::close(handoff);
            if has_slash {
                libc::execv(argv_ptrs[0], argv_ptrs.as_ptr());
            } else {
                libc::execvp(argv_ptrs[0], argv_ptrs.as_ptr());
            }
            let msg = b"launchpad netcap: the program could not start\n";
            libc::write(2, msg.as_ptr().cast(), msg.len());
            libc::_exit(127);
        }
    }
    CHILD.store(pid, Ordering::Relaxed);
    // SAFETY: plain system calls on descriptors this process owns.
    unsafe { libc::close(handoff) };
    // Processes that detach from their parent (crash handlers, daemons) come back to the supervisor instead of
    // the sandbox's init, so it stays their ancestor for Yama's ptrace rules.
    // SAFETY: sets a flag of this process.
    unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) };
    // SAFETY: as above.
    let listener = match unsafe { recv_fd(pair[0]) } {
        Some(fd) => fd,
        None => {
            let status = wait_child(pid);
            exit_like(status);
        }
    };
    // SAFETY: as above.
    unsafe {
        libc::write(pair[0], b"k".as_ptr().cast(), 1);
        libc::close(pair[0]);
        // Same-user processes cannot attach to the supervisor or read its memory.
        libc::prctl(libc::PR_SET_DUMPABLE, 0);
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC);
        if null >= 0 {
            libc::dup2(null, 0);
            libc::dup2(null, 1);
            libc::close(null);
        }
        for &sig in FORWARDED {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = forward as *const () as usize;
            sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
    let sup = Arc::new(Supervisor { listener, port, key, table: Mutex::new(Table::default()) });
    // Signals go to the main thread only: the threads below block them.
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: a local signal set.
    unsafe {
        libc::sigemptyset(&mut set);
        for &sig in FORWARDED {
            libc::sigaddset(&mut set, sig);
        }
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
    let s = sup.clone();
    std::thread::spawn(move || s.serve());
    // SAFETY: as above.
    unsafe { libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut()) };
    let status = wait_child(pid);
    exit_like(status)
}

/// How the supervisor reads the memory and descriptors of a process that made itself non-dumpable (Chrome,
/// crashpad, ssh-agent): `process_vm_readv`, `pidfd_getfd` and `/proc/<pid>/fd` then need `CAP_SYS_PTRACE` in the
/// user namespace of the process, and without them every checked `sendmsg` or `connect` would fail with EFAULT.
#[derive(Clone, Copy, PartialEq)]
enum Reach {
    /// The supervisor already has `CAP_SYS_PTRACE` (a privileged machine): nothing changes.
    Capable,
    /// The program starts in a new user namespace that the supervisor owns, so the supervisor has every
    /// capability over it from its own namespace, and the program drops all of them (`own_namespace`).
    Namespace,
    /// No user namespaces (HF Jobs refuse them; Ubuntu 24.04 removes their capabilities through AppArmor):
    /// `prctl(PR_SET_DUMPABLE, 0)` succeeds without effect, so processes stay readable.
    KeepDumpable,
}

impl Reach {
    fn detect() -> Reach {
        // SAFETY: capget into locals.
        let mut head = CapHeader { version: CAP_V3, pid: 0 };
        let mut data = [CapData::default(); 2];
        if unsafe { libc::syscall(libc::SYS_capget, &mut head, data.as_mut_ptr()) } != 0 {
            return Reach::KeepDumpable;
        }
        let ptrace = 19; // CAP_SYS_PTRACE
        if data[ptrace / 32].effective & (1 << (ptrace % 32)) != 0 {
            return Reach::Capable;
        }
        // A program with capabilities would lose them in a new namespace.
        if data.iter().any(|d| d.permitted != 0) {
            return Reach::KeepDumpable;
        }
        // Ubuntu 24.04 confines an unconfined process that creates a namespace to a profile without capabilities.
        if std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns").is_ok_and(|v| v.trim() == "1") {
            return Reach::KeepDumpable;
        }
        Reach::Namespace
    }
}

const CAP_V3: u32 = 0x2008_0522;

#[repr(C)]
struct CapHeader {
    version: u32,
    pid: libc::c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

/// In the child, before exec: moves it to a new user namespace mapping only its own user and group, then drops
/// every capability it has there (bounding set included, so no exec regains one). Whether the namespace holds:
/// false when the kernel refuses it, the program then keeps its namespace and the filter keeps it dumpable.
/// SAFETY: async-signal-safe calls only, on memory prepared before fork.
unsafe fn own_namespace(handoff: RawFd, uid_map: &[u8], gid_map: &[u8]) -> bool {
    unsafe {
        if libc::unshare(libc::CLONE_NEWUSER) != 0 {
            return false;
        }
        let write = |path: &std::ffi::CStr, data: &[u8]| {
            let fd = libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
            let ok = fd >= 0 && libc::write(fd, data.as_ptr().cast(), data.len()) == data.len() as isize;
            if fd >= 0 {
                libc::close(fd);
            }
            ok
        };
        // Without a mapping the program would run as the overflow user: never start it like that.
        if !write(c"/proc/self/uid_map", uid_map) {
            child_fail(handoff, b"user namespace refused its user mapping\n");
        }
        write(c"/proc/self/setgroups", b"deny");
        if !write(c"/proc/self/gid_map", gid_map) {
            child_fail(handoff, b"user namespace refused its group mapping\n");
        }
        // A refused drop means the capabilities are not usable here (an AppArmor profile): the supervisor's
        // probably are not either.
        let mut usable = true;
        for cap in 0..64 {
            if libc::prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0) != 0 {
                usable &= errno() == libc::EINVAL && cap > 0;
                break;
            }
        }
        libc::prctl(libc::PR_CAP_AMBIENT, libc::PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0);
        let mut head = CapHeader { version: CAP_V3, pid: 0 };
        let data = [CapData::default(); 2];
        if libc::syscall(libc::SYS_capset, &mut head, data.as_ptr()) != 0 {
            child_fail(handoff, b"capabilities could not be dropped\n");
        }
        usable
    }
}

/// SAFETY: async-signal-safe calls only (used in the child after fork).
unsafe fn child_fail(handoff: RawFd, msg: &[u8]) -> ! {
    unsafe {
        let head = b"launchpad netcap: ";
        libc::write(2, head.as_ptr().cast(), head.len());
        libc::write(2, msg.as_ptr().cast(), msg.len());
        libc::close(handoff);
        libc::_exit(126);
    }
}

/// Sends a descriptor over a Unix socket (SCM_RIGHTS), without allocating.
unsafe fn send_fd(sock: RawFd, fd: RawFd) {
    unsafe {
        let mut byte = [b'L'];
        let mut iov = libc::iovec { iov_base: byte.as_mut_ptr().cast(), iov_len: 1 };
        let mut space = [0u8; 64];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = space.as_mut_ptr().cast();
        msg.msg_controllen = libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) as _;
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>(), fd);
        libc::sendmsg(sock, &msg, 0);
    }
}

unsafe fn recv_fd(sock: RawFd) -> Option<RawFd> {
    unsafe {
        let mut byte = [0u8; 1];
        let mut iov = libc::iovec { iov_base: byte.as_mut_ptr().cast(), iov_len: 1 };
        let mut space = [0u8; 64];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = space.as_mut_ptr().cast();
        msg.msg_controllen = space.len() as _;
        if libc::recvmsg(sock, &mut msg, libc::MSG_CMSG_CLOEXEC) <= 0 {
            return None;
        }
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        if cmsg.is_null() || (*cmsg).cmsg_type != libc::SCM_RIGHTS {
            return None;
        }
        Some(std::ptr::read_unaligned(libc::CMSG_DATA(cmsg).cast::<RawFd>()))
    }
}

/// Waits for the program; the orphans it leaves behind (the supervisor is their subreaper) are reaped meanwhile.
fn wait_child(pid: libc::pid_t) -> libc::c_int {
    let mut status = 0;
    loop {
        // SAFETY: waits for this process's own children.
        let r = unsafe { libc::waitpid(-1, &mut status, 0) };
        if r == pid {
            return status;
        }
        if r < 0 && errno() != libc::EINTR {
            return 127 << 8;
        }
    }
}

/// Exits with the program's status: the same code, or the same signal.
fn exit_like(status: libc::c_int) -> ! {
    if libc::WIFSIGNALED(status) {
        let sig = libc::WTERMSIG(status);
        // SAFETY: restores the default action and re-raises the signal on this process.
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, sig);
            libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
            libc::kill(libc::getpid(), sig);
        }
        std::process::exit(128 + sig);
    }
    std::process::exit(libc::WEXITSTATUS(status))
}

// ------------------------------------------------------------------ notifications

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Tcp,
    Udp,
}

/// Sockets the supervisor created, by inode.
#[derive(Default)]
struct Table {
    kinds: HashMap<u64, Kind>,
    order: VecDeque<u64>,
    /// Copies of TCP sockets that have not connected yet.
    copies: HashMap<u64, (OwnedFd, Instant)>,
    /// Original destinations of redirected sockets (for `getpeername`).
    peers: HashMap<u64, Vec<u8>>,
    peer_order: VecDeque<u64>,
}

const MAX_TRACKED: usize = 65_536;
/// A TCP socket that has not connected after this long is no longer redirected (its copy would keep it open).
const COPY_TTL: Duration = Duration::from_secs(600);

impl Table {
    fn add(&mut self, ino: u64, kind: Kind, copy: Option<OwnedFd>) {
        self.peers.remove(&ino);
        self.copies.remove(&ino);
        if self.kinds.insert(ino, kind).is_none() {
            self.order.push_back(ino);
        }
        while self.order.len() > MAX_TRACKED {
            if let Some(old) = self.order.pop_front() {
                self.kinds.remove(&old);
                self.copies.remove(&old);
            }
        }
        if let Some(c) = copy {
            self.copies.insert(ino, (c, Instant::now()));
        }
        if self.copies.len() > 256 {
            self.copies.retain(|_, (_, at)| at.elapsed() < COPY_TTL);
        }
    }

    fn add_peer(&mut self, ino: u64, addr: Vec<u8>) {
        if self.peers.insert(ino, addr).is_none() {
            self.peer_order.push_back(ino);
        }
        while self.peer_order.len() > MAX_TRACKED {
            if let Some(old) = self.peer_order.pop_front() {
                self.peers.remove(&old);
            }
        }
    }
}

struct Supervisor {
    listener: RawFd,
    port: u16,
    key: String,
    table: Mutex<Table>,
}

enum Reply {
    Val(i64),
    Err(i32),
    Continue,
    /// Already answered (or the notification is gone).
    None,
}

impl Supervisor {
    fn serve(self: Arc<Self>) {
        loop {
            let mut pfd = libc::pollfd { fd: self.listener, events: libc::POLLIN, revents: 0 };
            // SAFETY: polls a descriptor this process owns.
            let r = unsafe { libc::poll(&mut pfd, 1, -1) };
            if r < 0 {
                if errno() == libc::EINTR {
                    continue;
                }
                return;
            }
            if pfd.revents & libc::POLLIN == 0 {
                // POLLHUP: no process uses the filter anymore.
                if pfd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
                    return;
                }
                continue;
            }
            // SAFETY: the kernel fills a zeroed seccomp_notif.
            let mut n: libc::seccomp_notif = unsafe { std::mem::zeroed() };
            // SAFETY: as above.
            if unsafe { libc::ioctl(self.listener, libc::SECCOMP_IOCTL_NOTIF_RECV, &mut n) } != 0 {
                continue;
            }
            let reply = self.clone().handle(&n);
            self.respond(n.id, reply);
        }
    }

    fn respond(&self, id: u64, reply: Reply) {
        let (val, error, flags) = match reply {
            Reply::Val(v) => (v, 0, 0),
            Reply::Err(e) => (0, -e, 0),
            Reply::Continue => (0, 0, libc::SECCOMP_USER_NOTIF_FLAG_CONTINUE as u32),
            Reply::None => return,
        };
        let mut resp = libc::seccomp_notif_resp { id, val, error, flags };
        // SAFETY: answers a notification of this listener; a gone notification fails harmlessly.
        unsafe { libc::ioctl(self.listener, libc::SECCOMP_IOCTL_NOTIF_SEND, &mut resp) };
    }

    /// Whether the notification is still pending (its process did not exit and its PID was not reused).
    fn valid(&self, id: u64) -> bool {
        let mut id = id;
        // SAFETY: as above.
        unsafe { libc::ioctl(self.listener, libc::SECCOMP_IOCTL_NOTIF_ID_VALID, &mut id) == 0 }
    }

    fn handle(self: Arc<Self>, n: &libc::seccomp_notif) -> Reply {
        let a = n.data.args;
        let nr = n.data.nr as libc::c_long;
        if nr == libc::SYS_socket {
            self.socket(n.id, a[0] as i32, a[1] as i32, a[2] as i32)
        } else if nr == libc::SYS_connect {
            self.connect(n)
        } else if nr == libc::SYS_bind {
            self.bind(n)
        } else if nr == libc::SYS_sendto {
            self.send_check(n, a[0], a[4], a[5] as usize)
        } else if nr == libc::SYS_sendmsg {
            match read_mem(n.pid, a[1], 16) {
                Some(h) => {
                    let (name, len) = (u64::from_ne_bytes(h[0..8].try_into().unwrap()), u32::from_ne_bytes(h[8..12].try_into().unwrap()));
                    if name == 0 || len == 0 { Reply::Continue } else { self.send_check(n, a[0], name, len as usize) }
                }
                None => Reply::Err(libc::EFAULT),
            }
        } else if nr == libc::SYS_sendmmsg {
            let count = (a[2] as usize).min(1024);
            match read_mem(n.pid, a[1], count * 64) {
                Some(v) => {
                    for m in v.chunks(64) {
                        let (name, len) = (u64::from_ne_bytes(m[0..8].try_into().unwrap()), u32::from_ne_bytes(m[8..12].try_into().unwrap()));
                        if name != 0 && len != 0 && let Reply::Err(e) = self.send_check(n, a[0], name, len as usize) {
                            return Reply::Err(e);
                        }
                    }
                    Reply::Continue
                }
                None => Reply::Err(libc::EFAULT),
            }
        } else if nr == libc::SYS_getpeername {
            self.getpeername(n)
        } else if nr == libc::SYS_listen {
            // A listening socket never connects: its copy goes.
            if let Ok(ino) = self.inode(n, a[0]) {
                self.table.lock().unwrap().copies.remove(&ino);
            }
            Reply::Continue
        } else {
            Reply::Continue
        }
    }

    fn socket(&self, id: u64, domain: i32, ty: i32, proto: i32) -> Reply {
        let base = ty & 0xf;
        let kind = match (base, proto) {
            (libc::SOCK_STREAM, 0 | libc::IPPROTO_TCP | libc::IPPROTO_MPTCP) => Kind::Tcp,
            (libc::SOCK_DGRAM, 0 | libc::IPPROTO_UDP | libc::IPPROTO_UDPLITE) => Kind::Udp,
            (libc::SOCK_DGRAM, libc::IPPROTO_ICMP | libc::IPPROTO_ICMPV6) => return Reply::Err(libc::EACCES),
            (libc::SOCK_RAW, _) => return Reply::Err(libc::EPERM),
            _ => return Reply::Err(libc::EPROTONOSUPPORT),
        };
        // SAFETY: creates a socket for the agent with the flags it asked for.
        let fd = unsafe { libc::socket(domain, ty | libc::SOCK_CLOEXEC, proto) };
        if fd < 0 {
            return Reply::Err(errno());
        }
        // SAFETY: fd was just created by this process.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut add = libc::seccomp_notif_addfd { id, flags: 0, srcfd: fd.as_raw_fd() as u32, newfd: 0, newfd_flags: if ty & libc::SOCK_CLOEXEC != 0 { libc::O_CLOEXEC as u32 } else { 0 } };
        // SAFETY: installs a copy of fd in the notifying process.
        let target = unsafe { libc::ioctl(self.listener, libc::SECCOMP_IOCTL_NOTIF_ADDFD, &mut add) };
        if target < 0 {
            return match errno() {
                libc::ENOENT => Reply::None,
                e => Reply::Err(e),
            };
        }
        let Some(ino) = fd_inode(fd.as_raw_fd()) else { return Reply::Val(target as i64) };
        let copy = (kind == Kind::Tcp).then_some(fd);
        self.table.lock().unwrap().add(ino, kind, copy);
        Reply::Val(target as i64)
    }

    /// `sendto`, `sendmsg` and `sendmmsg` with an address: UDP reaches port 53 only.
    fn send_check(&self, n: &libc::seccomp_notif, fd: u64, addr: u64, len: usize) -> Reply {
        let Some(bytes) = read_mem(n.pid, addr, len.min(128)) else { return Reply::Err(libc::EFAULT) };
        if policy::unix_target(&bytes) == Some(policy::UnixTarget::Abstract) {
            return Reply::Err(libc::EACCES);
        }
        let dest = match parse_addr(&bytes) {
            Ok(Some(d)) => d,
            Ok(None) => return Reply::Continue,
            Err(e) => return Reply::Err(e),
        };
        let ino = match self.inode(n, fd) {
            Ok(i) => i,
            Err(e) => return Reply::Err(e),
        };
        match self.table.lock().unwrap().kinds.get(&ino) {
            Some(Kind::Udp) if !policy::udp_allowed(dest.port()) => Reply::Err(libc::EACCES),
            _ => Reply::Continue,
        }
    }

    fn connect(self: Arc<Self>, n: &libc::seccomp_notif) -> Reply {
        let (fd, addr, len) = (n.data.args[0], n.data.args[1], n.data.args[2] as usize);
        if !(2..=128).contains(&len) {
            return Reply::Err(libc::EINVAL);
        }
        let Some(bytes) = read_mem(n.pid, addr, len) else { return Reply::Err(libc::EFAULT) };
        let ino = match self.inode(n, fd) {
            Ok(i) => i,
            Err(e) => return Reply::Err(e),
        };
        let kind = self.table.lock().unwrap().kinds.get(&ino).copied();
        let dest = match (kind, parse_addr(&bytes)) {
            (_, Err(e)) => return Reply::Err(e),
            // Unix and netlink sockets, and internet sockets the supervisor no longer tracks.
            (None, _) => return self.unchecked_socket(n, fd, bytes, false),
            (Some(_), Ok(Some(d))) => d,
            // Disconnecting a UDP socket (AF_UNSPEC).
            (Some(Kind::Udp), Ok(None)) => return Reply::Continue,
            (Some(Kind::Tcp), Ok(None)) => return Reply::Err(libc::EAFNOSUPPORT),
        };
        if !self.valid(n.id) {
            return Reply::None;
        }
        if policy::refused(dest.ip(), dest.port()) {
            return Reply::Err(libc::EACCES);
        }
        let mut table = self.table.lock().unwrap();
        if kind == Some(Kind::Udp) {
            return if policy::udp_allowed(dest.port()) { Reply::Continue } else { Reply::Err(libc::EACCES) };
        }
        if table.peers.contains_key(&ino) {
            // A connect restarted after a signal, or a second connect: the socket is already connected.
            return Reply::Err(libc::EISCONN);
        }
        let Some((copy, _)) = table.copies.remove(&ino) else { return Reply::Err(libc::EACCES) };
        drop(table);
        let family = u16::from_ne_bytes([bytes[0], bytes[1]]) as i32;
        let id = n.id;
        // Connects can wait on the network: each runs on its own thread.
        std::thread::spawn(move || {
            let result = if policy::goes_direct(dest.ip()) {
                connect_to(copy.as_raw_fd(), &dest)
            } else {
                let r = self.redirect(copy.as_raw_fd(), family, &dest);
                if r == 0 {
                    self.table.lock().unwrap().add_peer(ino, bytes);
                }
                r
            };
            drop(copy);
            self.respond(id, if result == 0 { Reply::Val(0) } else { Reply::Err(result) });
        });
        Reply::None
    }

    /// `bind`: internet sockets of the session as asked; Unix sockets through `unchecked_socket`.
    fn bind(self: Arc<Self>, n: &libc::seccomp_notif) -> Reply {
        let (fd, addr, len) = (n.data.args[0], n.data.args[1], n.data.args[2] as usize);
        if !(2..=128).contains(&len) {
            return Reply::Err(libc::EINVAL);
        }
        let Some(bytes) = read_mem(n.pid, addr, len) else { return Reply::Err(libc::EFAULT) };
        let ino = match self.inode(n, fd) {
            Ok(i) => i,
            Err(e) => return Reply::Err(e),
        };
        if self.table.lock().unwrap().kinds.contains_key(&ino) {
            return Reply::Continue;
        }
        self.unchecked_socket(n, fd, bytes, true)
    }

    /// `connect` or `bind` of a socket the supervisor did not create. A Unix socket gets it from the supervisor, on
    /// a copy of the descriptor with the address checked here (never the abstract namespace; a relative path is
    /// resolved in the agent's folder), so changing the address in memory after the check changes nothing. An
    /// internet socket the supervisor does not track is refused; other families (netlink) continue. Where the
    /// descriptor cannot be copied (HF Jobs refuse `pidfd_getfd`), the address is checked in place.
    fn unchecked_socket(self: Arc<Self>, n: &libc::seccomp_notif, fd: u64, bytes: Vec<u8>, bind: bool) -> Reply {
        let target = policy::unix_target(&bytes);
        if target == Some(policy::UnixTarget::Abstract) {
            return Reply::Err(libc::EACCES);
        }
        let copy = match self.copy_fd(n, fd) {
            Ok(c) => c,
            Err(libc::EPERM | libc::ENOSYS) => return if parse_addr(&bytes).is_ok_and(|d| d.is_some()) { Reply::Err(libc::EACCES) } else { Reply::Continue },
            Err(e) => return Reply::Err(e),
        };
        let mut domain: libc::c_int = 0;
        let mut l = 4 as libc::socklen_t;
        // SAFETY: reads an option of the supervisor's copy into a local.
        if unsafe { libc::getsockopt(copy.as_raw_fd(), libc::SOL_SOCKET, libc::SO_DOMAIN, (&mut domain as *mut libc::c_int).cast(), &mut l) } != 0 {
            return Reply::Err(errno());
        }
        match domain {
            libc::AF_UNIX => {}
            libc::AF_INET | libc::AF_INET6 => return Reply::Err(libc::EACCES),
            _ => return Reply::Continue,
        }
        // The address the supervisor uses: a relative path through a descriptor of its folder in the agent's view.
        let mut dir: Option<OwnedFd> = None;
        let addr = match target {
            Some(policy::UnixTarget::Path(path)) if path.first() != Some(&b'/') => {
                let (parent, name) = match path.iter().rposition(|b| *b == b'/') {
                    Some(i) => (&path[..i], &path[i + 1..]),
                    None => (&b"."[..], &path[..]),
                };
                let Ok(at) = CString::new([format!("/proc/{}/cwd/", n.pid).as_bytes(), parent].concat()) else { return Reply::Err(libc::EINVAL) };
                // SAFETY: opens a folder path from a local C string.
                let d = unsafe { libc::open(at.as_ptr(), libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC) };
                if d < 0 {
                    return Reply::Err(errno());
                }
                // SAFETY: d was just opened by this process.
                let d = unsafe { OwnedFd::from_raw_fd(d) };
                let full = [format!("/proc/self/fd/{}/", d.as_raw_fd()).as_bytes(), name].concat();
                dir = Some(d);
                if full.len() >= 108 {
                    return Reply::Err(libc::ENAMETOOLONG);
                }
                [&(libc::AF_UNIX as u16).to_ne_bytes()[..], &full, &[0]].concat()
            }
            _ => bytes,
        };
        if !self.valid(n.id) {
            return Reply::None;
        }
        let id = n.id;
        let run = move || {
            // SAFETY: the address is a local buffer of its length; the copy and the folder stay open meanwhile.
            let r = unsafe {
                let p = addr.as_ptr().cast::<libc::sockaddr>();
                let l = addr.len() as libc::socklen_t;
                if bind { libc::bind(copy.as_raw_fd(), p, l) } else { libc::connect(copy.as_raw_fd(), p, l) }
            };
            let reply = if r == 0 { Reply::Val(0) } else { Reply::Err(errno()) };
            drop(dir);
            reply
        };
        if bind {
            return run();
        }
        // A connect to a busy listener can wait: on its own thread.
        std::thread::spawn(move || {
            let reply = run();
            self.respond(id, reply);
        });
        Reply::None
    }

    /// A copy of a descriptor of the notifying process (`pidfd_getfd`), once the notification proves the PID
    /// still names it.
    fn copy_fd(&self, n: &libc::seccomp_notif, fd: u64) -> Result<OwnedFd, i32> {
        // SAFETY: opens a pidfd and copies one descriptor into this process; both are owned below.
        unsafe {
            let pidfd = libc::syscall(libc::SYS_pidfd_open, n.pid as libc::pid_t, 0) as RawFd;
            if pidfd < 0 {
                return Err(errno());
            }
            let pidfd = OwnedFd::from_raw_fd(pidfd);
            if !self.valid(n.id) {
                return Err(libc::ESRCH);
            }
            let copy = libc::syscall(libc::SYS_pidfd_getfd, pidfd.as_raw_fd(), fd as libc::c_int, 0) as RawFd;
            if copy < 0 {
                return Err(errno());
            }
            Ok(OwnedFd::from_raw_fd(copy))
        }
    }

    /// Connects the agent's socket to the proxy's capture port, which connects to `dest`. 0 or an errno.
    fn redirect(&self, fd: RawFd, family: i32, dest: &SocketAddr) -> i32 {
        let deadline = Instant::now() + Duration::from_secs(90);
        let proxy = if family == libc::AF_INET6 {
            let off: libc::c_int = 0;
            // SAFETY: an option on the supervisor's copy, before it connects.
            unsafe { libc::setsockopt(fd, libc::IPPROTO_IPV6, libc::IPV6_V6ONLY, (&off as *const libc::c_int).cast(), 4) };
            SocketAddr::new(IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped()), self.port)
        } else {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port)
        };
        let e = connect_to(fd, &proxy);
        if e != 0 && e != libc::EINPROGRESS {
            return e;
        }
        if e == libc::EINPROGRESS {
            if !wait_fd(fd, libc::POLLOUT, deadline) {
                return libc::ETIMEDOUT;
            }
            let mut err: libc::c_int = 0;
            let mut l = 4 as libc::socklen_t;
            // SAFETY: reads the pending error of the supervisor's copy.
            unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_ERROR, (&mut err as *mut libc::c_int).cast(), &mut l) };
            if err != 0 {
                return err;
            }
        }
        let header = policy::capture_header(&self.key, dest);
        let mut sent = 0;
        while sent < header.len() {
            // SAFETY: writes from a local buffer.
            let r = unsafe { libc::send(fd, header.as_ptr().add(sent).cast(), header.len() - sent, libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT) };
            if r > 0 {
                sent += r as usize;
            } else if r < 0 && errno() == libc::EAGAIN {
                if !wait_fd(fd, libc::POLLOUT, deadline) {
                    return libc::ETIMEDOUT;
                }
            } else {
                return libc::ECONNRESET;
            }
        }
        // The answer line, one byte at a time: what follows belongs to the agent.
        let mut line = vec![];
        while line.len() < 64 {
            if !wait_fd(fd, libc::POLLIN, deadline) {
                return libc::ETIMEDOUT;
            }
            let mut b = 0u8;
            // SAFETY: reads one byte into a local.
            let r = unsafe { libc::recv(fd, (&mut b as *mut u8).cast(), 1, libc::MSG_DONTWAIT) };
            if r == 1 {
                if b == b'\n' {
                    return policy::parse_capture_answer(&String::from_utf8_lossy(&line));
                }
                line.push(b);
            } else if r == 0 || errno() != libc::EAGAIN {
                return libc::ECONNRESET;
            }
        }
        libc::ECONNRESET
    }

    /// The socket inode behind a descriptor of the notifying process. A non-dumpable process's `/proc/<pid>/fd`
    /// belongs to root, which the supervisor's namespace does not map: a copy of the descriptor tells then.
    fn inode(&self, n: &libc::seccomp_notif, fd: u64) -> Result<u64, i32> {
        let e = match proc_inode(n.pid, fd) {
            Err(e @ (libc::EACCES | libc::EPERM)) => e,
            other => return other,
        };
        // SAFETY: opens a pidfd, then (once the notification proves the PID still names the notifying process)
        // copies the descriptor into this process; both are closed below.
        unsafe {
            let pidfd = libc::syscall(libc::SYS_pidfd_open, n.pid as libc::pid_t, 0) as RawFd;
            if pidfd < 0 {
                return Err(e);
            }
            let pidfd = OwnedFd::from_raw_fd(pidfd);
            if !self.valid(n.id) {
                return Err(libc::ESRCH);
            }
            let copy = libc::syscall(libc::SYS_pidfd_getfd, pidfd.as_raw_fd(), fd as libc::c_int, 0) as RawFd;
            if copy < 0 {
                return Err(match errno() {
                    libc::EBADF => libc::EBADF,
                    _ => e,
                });
            }
            let copy = OwnedFd::from_raw_fd(copy);
            let mut st: libc::stat = std::mem::zeroed();
            if libc::fstat(copy.as_raw_fd(), &mut st) != 0 {
                return Err(e);
            }
            if st.st_mode & libc::S_IFMT != libc::S_IFSOCK {
                return Err(libc::ENOTSOCK);
            }
            Ok(st.st_ino)
        }
    }

    fn getpeername(&self, n: &libc::seccomp_notif) -> Reply {
        let (fd, addr, lenp) = (n.data.args[0], n.data.args[1], n.data.args[2]);
        let Ok(ino) = self.inode(n, fd) else { return Reply::Continue };
        let Some(original) = self.table.lock().unwrap().peers.get(&ino).cloned() else { return Reply::Continue };
        let Some(len) = read_mem(n.pid, lenp, 4).map(|b| u32::from_ne_bytes(b[..4].try_into().unwrap()) as usize) else { return Reply::Err(libc::EFAULT) };
        if !self.valid(n.id) {
            return Reply::None;
        }
        let take = len.min(original.len());
        if (take > 0 && !write_mem(n.pid, addr, &original[..take])) || !write_mem(n.pid, lenp, &(original.len() as u32).to_ne_bytes()) {
            return Reply::Err(libc::EFAULT);
        }
        Reply::Val(0)
    }
}

/// connect(2) on a raw descriptor. 0, or an errno (EINPROGRESS for a non-blocking socket).
fn connect_to(fd: RawFd, dest: &SocketAddr) -> i32 {
    let (storage, len) = sockaddr(dest);
    loop {
        // SAFETY: storage holds a valid sockaddr of len bytes.
        let r = unsafe { libc::connect(fd, (&storage as *const libc::sockaddr_storage).cast(), len) };
        if r == 0 {
            return 0;
        }
        match errno() {
            libc::EINTR => continue,
            libc::EISCONN => return 0,
            e => return e,
        }
    }
}

fn wait_fd(fd: RawFd, events: libc::c_short, deadline: Instant) -> bool {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        let mut pfd = libc::pollfd { fd, events, revents: 0 };
        // SAFETY: polls a descriptor this process owns.
        let r = unsafe { libc::poll(&mut pfd, 1, left.as_millis().min(i32::MAX as u128) as i32) };
        if r > 0 {
            return true;
        }
        if r < 0 && errno() != libc::EINTR {
            return false;
        }
    }
}

fn sockaddr(a: &SocketAddr) -> (libc::sockaddr_storage, libc::socklen_t) {
    // SAFETY: a zeroed sockaddr_storage is valid, and both address types fit in it.
    let mut s: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    match a {
        SocketAddr::V4(v4) => {
            let sin = libc::sockaddr_in { sin_family: libc::AF_INET as u16, sin_port: v4.port().to_be(), sin_addr: libc::in_addr { s_addr: u32::from_ne_bytes(v4.ip().octets()) }, sin_zero: [0; 8] };
            unsafe { std::ptr::write((&mut s as *mut libc::sockaddr_storage).cast(), sin) };
            (s, std::mem::size_of::<libc::sockaddr_in>() as u32)
        }
        SocketAddr::V6(v6) => {
            let sin6 = libc::sockaddr_in6 { sin6_family: libc::AF_INET6 as u16, sin6_port: v6.port().to_be(), sin6_flowinfo: v6.flowinfo(), sin6_addr: libc::in6_addr { s6_addr: v6.ip().octets() }, sin6_scope_id: v6.scope_id() };
            unsafe { std::ptr::write((&mut s as *mut libc::sockaddr_storage).cast(), sin6) };
            (s, std::mem::size_of::<libc::sockaddr_in6>() as u32)
        }
    }
}

/// An internet address, None for other families, or EINVAL for a short one.
fn parse_addr(b: &[u8]) -> Result<Option<SocketAddr>, i32> {
    if b.len() < 2 {
        return Err(libc::EINVAL);
    }
    let family = u16::from_ne_bytes([b[0], b[1]]) as i32;
    match family {
        libc::AF_INET => {
            if b.len() < 16 {
                return Err(libc::EINVAL);
            }
            let ip = Ipv4Addr::new(b[4], b[5], b[6], b[7]);
            Ok(Some(SocketAddr::new(IpAddr::V4(ip), u16::from_be_bytes([b[2], b[3]]))))
        }
        libc::AF_INET6 => {
            if b.len() < 24 {
                return Err(libc::EINVAL);
            }
            let ip = Ipv6Addr::from(<[u8; 16]>::try_from(&b[8..24]).unwrap());
            let scope = if b.len() >= 28 { u32::from_ne_bytes(b[24..28].try_into().unwrap()) } else { 0 };
            Ok(Some(SocketAddr::V6(std::net::SocketAddrV6::new(ip, u16::from_be_bytes([b[2], b[3]]), 0, scope))))
        }
        _ => Ok(None),
    }
}

fn read_mem(pid: u32, addr: u64, len: usize) -> Option<Vec<u8>> {
    if addr == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    let local = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: len };
    let remote = libc::iovec { iov_base: addr as *mut libc::c_void, iov_len: len };
    // SAFETY: copies from the notifying process into a local buffer of len bytes.
    let n = unsafe { libc::process_vm_readv(pid as libc::pid_t, &local, 1, &remote, 1, 0) };
    (n == len as isize).then_some(buf)
}

fn write_mem(pid: u32, addr: u64, data: &[u8]) -> bool {
    if addr == 0 {
        return false;
    }
    let local = libc::iovec { iov_base: data.as_ptr() as *mut libc::c_void, iov_len: data.len() };
    let remote = libc::iovec { iov_base: addr as *mut libc::c_void, iov_len: data.len() };
    // SAFETY: copies a local buffer into the notifying process, at the address its system call named.
    let n = unsafe { libc::process_vm_writev(pid as libc::pid_t, &local, 1, &remote, 1, 0) };
    n == data.len() as isize
}

/// The socket inode behind a descriptor of the agent, from `/proc/<pid>/fd`.
fn proc_inode(pid: u32, fd: u64) -> Result<u64, i32> {
    let link = std::fs::read_link(format!("/proc/{pid}/fd/{fd}")).map_err(|e| match e.raw_os_error() {
        Some(libc::ENOENT) => libc::EBADF,
        Some(e) => e,
        None => libc::EBADF,
    })?;
    let s = link.to_string_lossy();
    s.strip_prefix("socket:[").and_then(|r| r.strip_suffix(']')).and_then(|n| n.parse().ok()).ok_or(libc::ENOTSOCK)
}

fn fd_inode(fd: RawFd) -> Option<u64> {
    // SAFETY: fstat into a local.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    (unsafe { libc::fstat(fd, &mut st) } == 0).then_some(st.st_ino)
}

// ------------------------------------------------------------------ self-test

/// The probe's checks as Chrome's crash handler would see them: from a non-dumpable process that detached from
/// its parent (a double fork), a Unix socket works, UDP other than DNS stays refused, and a TCP connect to `dest`
/// still goes through the capture.
fn nondumpable(dest: SocketAddr) -> Vec<serde_json::Value> {
    let failed = |detail: &str| vec![serde_json::json!({"id": "nondumpable_unix", "ok": false, "detail": detail})];
    let mut pipe = [0; 2];
    // SAFETY: a pipe, then a double fork in this single-threaded probe: the grandchild runs the checks, writes
    // them to the pipe and exits; the parent reads them.
    unsafe {
        if libc::pipe(pipe.as_mut_ptr()) != 0 {
            return failed("pipe");
        }
        let child = libc::fork();
        if child == 0 {
            libc::close(pipe[0]);
            let parent = libc::getpid();
            if libc::fork() == 0 {
                // Detached once its parent exited.
                for _ in 0..500 {
                    if libc::getppid() != parent {
                        break;
                    }
                    libc::usleep(2_000);
                }
                let out = serde_json::Value::Array(orphan_checks(dest)).to_string();
                libc::write(pipe[1], out.as_ptr().cast(), out.len());
                libc::_exit(0);
            }
            libc::_exit(0);
        }
        libc::close(pipe[1]);
        if child < 0 {
            libc::close(pipe[0]);
            return failed("fork");
        }
        libc::waitpid(child, std::ptr::null_mut(), 0);
        let mut out = vec![];
        let mut buf = [0u8; 4096];
        loop {
            let n = libc::read(pipe[0], buf.as_mut_ptr().cast(), buf.len());
            if n <= 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        libc::close(pipe[0]);
        serde_json::from_slice(&out).unwrap_or_else(|_| failed("the detached process gave no answer"))
    }
}

fn orphan_checks(dest: SocketAddr) -> Vec<serde_json::Value> {
    let mut checks = vec![];
    let mut add = |id: &str, ok: bool, detail: String| checks.push(serde_json::json!({"id": id, "ok": ok, "detail": detail}));
    // SAFETY: changes this process's own flag.
    unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    // SAFETY: reads this process's own flag.
    let state = if unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) } == 0 { "non-dumpable" } else { "kept dumpable by the capture" };
    // The crash handler's handshake: sendmsg on a Unix socket pair.
    let mut pair = [0; 2];
    // SAFETY: a socket pair and a one-byte sendmsg from locals; the pair is closed below.
    let sent = unsafe {
        if libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0, pair.as_mut_ptr()) != 0 {
            Err(errno())
        } else {
            let mut byte = [b'x'];
            let mut iov = libc::iovec { iov_base: byte.as_mut_ptr().cast(), iov_len: 1 };
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            let r = if libc::sendmsg(pair[0], &msg, 0) == 1 { Ok(()) } else { Err(errno()) };
            libc::close(pair[0]);
            libc::close(pair[1]);
            r
        }
    };
    add("nondumpable_unix", sent.is_ok(), match sent {
        Ok(()) => format!("sendmsg on a Unix socket works ({state})"),
        Err(e) => format!("sendmsg on a Unix socket failed: {}", std::io::Error::from_raw_os_error(e)),
    });
    match std::net::UdpSocket::bind("0.0.0.0:0") {
        Ok(u) => {
            let r = u.send_to(b"launchpad", "192.0.2.1:443").map_err(|e| e.raw_os_error());
            add("nondumpable_udp", r == Err(Some(libc::EACCES)), format!("UDP to port 443: {}", if r == Err(Some(libc::EACCES)) { "refused".into() } else { format!("{r:?}") }));
        }
        Err(e) => add("nondumpable_udp", false, format!("{e}")),
    }
    match std::net::TcpStream::connect_timeout(&dest, Duration::from_secs(20)) {
        Ok(s) => {
            let peer = s.peer_addr().map(|p| p.to_string()).unwrap_or_default();
            add("nondumpable_tcp", peer == dest.to_string(), format!("connected to {dest} (peer {peer})"));
        }
        Err(e) => add("nondumpable_tcp", false, format!("{dest}: {e}")),
    }
    checks
}

/// `launchpad netcap --self-test`: whether capture works on this machine (exit 0), by running the probe's quick
/// checks under a supervisor.
fn self_test() -> ! {
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| fail("no executable path"));
    run(1, "self-test".into(), &[exe, "netcap-probe".into(), "--quick".into()])
}

/// `launchpad netcap-probe [--quick] [--ca <file>] [--host <name>] [--nondumpable <ip:port>]`: checks run by a
/// process under capture, as a client that ignores proxy settings would see them; prints a JSON list of
/// `{id, ok, detail}`. The full probe ends with the checks of a non-dumpable process, connecting to `<host>:443`;
/// `--nondumpable` runs only those, connecting to the given address.
pub fn probe(args: &[String]) -> ! {
    let quick = args.iter().any(|a| a == "--quick");
    if let Some(dest) = args.iter().position(|a| a == "--nondumpable").and_then(|i| args.get(i + 1)) {
        let Ok(dest) = dest.parse::<SocketAddr>() else { fail("--nondumpable needs an ip:port") };
        let checks = nondumpable(dest);
        let ok = checks.iter().all(|c| c["ok"] == true);
        println!("{}", serde_json::Value::Array(checks));
        std::process::exit(if ok { 0 } else { 1 })
    }
    let ca = args.iter().position(|a| a == "--ca").and_then(|i| args.get(i + 1)).cloned();
    let host = args.iter().position(|a| a == "--host").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "huggingface.co".into());
    let mut checks: Vec<serde_json::Value> = vec![];
    let mut v4 = None;
    let mut add = |id: &str, ok: bool, detail: String| checks.push(serde_json::json!({"id": id, "ok": ok, "detail": detail}));
    // Sockets come from the supervisor.
    match std::net::UdpSocket::bind("0.0.0.0:0") {
        Ok(_) => add("socket", true, "internet sockets are created by the supervisor".into()),
        Err(e) => add("socket", false, format!("{e}")),
    }
    // UDP to another port than 53 is refused; DNS is allowed.
    match std::net::UdpSocket::bind("0.0.0.0:0") {
        Ok(u) => {
            let refused = u.send_to(b"launchpad", "192.0.2.1:443").map_err(|e| e.raw_os_error()) == Err(Some(libc::EACCES));
            add("udp_refused", refused, if refused { "UDP to port 443 is refused".into() } else { "UDP to port 443 was not refused".into() });
            let dns = u.connect("192.0.2.1:53").is_ok();
            add("udp_dns", dns, if dns { "UDP to port 53 is allowed".into() } else { "UDP to port 53 was refused".into() });
        }
        Err(e) => add("udp_refused", false, format!("{e}")),
    }
    // Unix sockets: a path works (bind and connect run in the supervisor), the abstract namespace is refused.
    let path = std::env::temp_dir().join(format!("lp-netcap-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let unix = std::os::unix::net::UnixListener::bind(&path).and_then(|l| std::os::unix::net::UnixStream::connect(&path).map(|_| l));
    let _ = std::fs::remove_file(&path);
    add("unix_path", unix.is_ok(), match &unix {
        Ok(_) => "a Unix socket at a path works".into(),
        Err(e) => format!("a Unix socket at a path failed: {e}"),
    });
    let abs = {
        use std::os::linux::net::SocketAddrExt;
        std::os::unix::net::SocketAddr::from_abstract_name(b"launchpad-netcap-check").and_then(|a| std::os::unix::net::UnixStream::connect_addr(&a))
    };
    let refused = abs.as_ref().err().and_then(|e| e.raw_os_error()) == Some(libc::EACCES);
    add("unix_abstract", refused, if refused { "abstract Unix sockets are refused".into() } else { format!("an abstract Unix socket was not refused: {:?}", abs.map(|_| ())) });
    // Link-local (cloud metadata) is refused before any connection.
    let meta = std::net::TcpStream::connect_timeout(&SocketAddr::from(([169, 254, 169, 254], 80)), Duration::from_secs(5));
    let refused = meta.as_ref().err().and_then(|e| e.raw_os_error()) == Some(libc::EACCES);
    add("link_local", refused, if refused { "link-local addresses (cloud metadata) are refused".into() } else { format!("169.254.169.254:80 was not refused: {:?}", meta.map(|_| ())) });
    // SAFETY: io_uring_setup with a zeroed parameter block; refused by the filter.
    let mut params = [0u8; 120];
    let r = unsafe { libc::syscall(libc::SYS_io_uring_setup, 1, params.as_mut_ptr()) };
    let blocked = r < 0 && errno() == libc::ENOSYS;
    if r >= 0 {
        // SAFETY: closes the ring it just created.
        unsafe { libc::close(r as RawFd) };
    }
    add("io_uring", blocked, if blocked { "io_uring is blocked".into() } else { "io_uring is available".into() });
    if !quick {
        // DNS, then a direct TCP connection that ignores any proxy setting, over IPv4 and IPv6.
        use std::net::ToSocketAddrs;
        match (host.as_str(), 443).to_socket_addrs() {
            Ok(addrs) => {
                let addrs: Vec<SocketAddr> = addrs.collect();
                v4 = addrs.iter().find(|a| a.is_ipv4()).copied();
                add("dns", !addrs.is_empty(), format!("{host}: {} addresses", addrs.len()));
                for (id, want_v6) in [("tcp_v4", false), ("tcp_v6", true)] {
                    match addrs.iter().find(|a| a.is_ipv6() == want_v6) {
                        Some(a) => match std::net::TcpStream::connect_timeout(a, Duration::from_secs(20)) {
                            Ok(s) => {
                                let peer = s.peer_addr().map(|p| p.to_string()).unwrap_or_default();
                                add(id, true, format!("connected to {a} (peer {peer})"));
                            }
                            Err(e) => add(id, false, format!("{a}: {e}")),
                        },
                        None => add(id, false, format!("{host} has no {} address", if want_v6 { "IPv6" } else { "IPv4" })),
                    }
                }
            }
            Err(e) => add("dns", false, format!("{host}: {e}")),
        }
        // The runner CA in the system certificate store.
        if let Some(ca) = ca {
            let pem = std::fs::read_to_string(&ca).unwrap_or_default();
            let body = pem.lines().filter(|l| !l.starts_with("-----")).collect::<String>();
            let found = policy::SYSTEM_BUNDLES.iter().find(|p| std::fs::read_to_string(p).is_ok_and(|t| !body.is_empty() && t.lines().filter(|l| !l.starts_with("-----")).collect::<String>().contains(&body)));
            add("ca_store", found.is_some(), match found {
                Some(p) => format!("the runner CA is in {p}"),
                None => "the runner CA is not in the system certificate store".into(),
            });
        }
    }
    if let Some(dest) = v4 {
        checks.extend(nondumpable(dest));
    }
    // The quick checks (the self-test) all pass; the full probe reports and the runner judges.
    let ok = !quick || checks.iter().all(|c| c["ok"] == true);
    println!("{}", serde_json::Value::Array(checks));
    std::process::exit(if ok { 0 } else { 1 })
}
