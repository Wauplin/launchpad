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
//! - Loopback connects go to their address. UDP reaches port 53 only. Other socket families, io_uring and TCP
//!   Fast Open are refused.
//!
//! It stays the agent's parent, so it may read the agent's memory under Yama's ptrace rules; it forwards the
//! signals sent to it and exits with the program's status. Processes still running after the program exits lose
//! their network calls (the listener closes with the supervisor).

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
/// supervisor before the program starts.
fn filter(handoff: RawFd) -> Vec<SockFilter> {
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
        Jeq(nr(libc::SYS_sendto), L("sendto"), Next),
        Jeq(nr(libc::SYS_sendmsg), L("sendmsg"), Next),
        Jeq(nr(libc::SYS_sendmmsg), L("sendmmsg"), Next),
        Jeq(nr(libc::SYS_getpeername), L("notif"), Next),
        Jeq(nr(libc::SYS_listen), L("notif"), Next),
        Jeq(nr(libc::SYS_io_uring_setup), L("enosys"), Next),
        Jeq(nr(libc::SYS_io_uring_enter), L("enosys"), Next),
        Jeq(nr(libc::SYS_io_uring_register), L("enosys"), Next),
        Ret(RET_ALLOW),
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
    let prog = filter(handoff);
    let fprog = SockFprog { len: prog.len() as u16, filter: prog.as_ptr() };
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
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                child_fail(handoff, b"no_new_privs refused\n");
            }
            let listener = libc::syscall(libc::SYS_seccomp, libc::SECCOMP_SET_MODE_FILTER, libc::SECCOMP_FILTER_FLAG_NEW_LISTENER, &fprog as *const SockFprog) as libc::c_int;
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

fn wait_child(pid: libc::pid_t) -> libc::c_int {
    let mut status = 0;
    loop {
        // SAFETY: waits for this process's own child.
        let r = unsafe { libc::waitpid(pid, &mut status, 0) };
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
        } else if nr == libc::SYS_sendto {
            self.send_check(n.pid, a[0], a[4], a[5] as usize)
        } else if nr == libc::SYS_sendmsg {
            match read_mem(n.pid, a[1], 16) {
                Some(h) => {
                    let (name, len) = (u64::from_ne_bytes(h[0..8].try_into().unwrap()), u32::from_ne_bytes(h[8..12].try_into().unwrap()));
                    if name == 0 || len == 0 { Reply::Continue } else { self.send_check(n.pid, a[0], name, len as usize) }
                }
                None => Reply::Err(libc::EFAULT),
            }
        } else if nr == libc::SYS_sendmmsg {
            let count = (a[2] as usize).min(1024);
            match read_mem(n.pid, a[1], count * 64) {
                Some(v) => {
                    for m in v.chunks(64) {
                        let (name, len) = (u64::from_ne_bytes(m[0..8].try_into().unwrap()), u32::from_ne_bytes(m[8..12].try_into().unwrap()));
                        if name != 0 && len != 0 && let Reply::Err(e) = self.send_check(n.pid, a[0], name, len as usize) {
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
            if let Ok(ino) = inode(n.pid, a[0]) {
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
    fn send_check(&self, pid: u32, fd: u64, addr: u64, len: usize) -> Reply {
        let Some(bytes) = read_mem(pid, addr, len.min(128)) else { return Reply::Err(libc::EFAULT) };
        let dest = match parse_addr(&bytes) {
            Ok(Some(d)) => d,
            Ok(None) => return Reply::Continue,
            Err(e) => return Reply::Err(e),
        };
        let ino = match inode(pid, fd) {
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
        let dest = match parse_addr(&bytes) {
            Ok(Some(d)) => d,
            // Unix, netlink and AF_UNSPEC addresses: unchanged.
            Ok(None) => return Reply::Continue,
            Err(e) => return Reply::Err(e),
        };
        let ino = match inode(n.pid, fd) {
            Ok(i) => i,
            Err(e) => return Reply::Err(e),
        };
        if !self.valid(n.id) {
            return Reply::None;
        }
        let mut table = self.table.lock().unwrap();
        match table.kinds.get(&ino).copied() {
            Some(Kind::Udp) => return if policy::udp_allowed(dest.port()) { Reply::Continue } else { Reply::Err(libc::EACCES) },
            Some(Kind::Tcp) => {}
            // Not an internet socket of this session (a Unix socket given an internet address): the kernel answers.
            None => return Reply::Continue,
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
            } else if r == 0 {
                return libc::ECONNRESET;
            } else if errno() != libc::EAGAIN {
                return libc::ECONNRESET;
            }
        }
        libc::ECONNRESET
    }

    fn getpeername(&self, n: &libc::seccomp_notif) -> Reply {
        let (fd, addr, lenp) = (n.data.args[0], n.data.args[1], n.data.args[2]);
        let Ok(ino) = inode(n.pid, fd) else { return Reply::Continue };
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

/// The socket inode behind a descriptor of the agent.
fn inode(pid: u32, fd: u64) -> Result<u64, i32> {
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

/// `launchpad netcap --self-test`: whether capture works on this machine (exit 0), by running the probe's quick
/// checks under a supervisor.
fn self_test() -> ! {
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| fail("no executable path"));
    run(1, "self-test".into(), &[exe, "netcap-probe".into(), "--quick".into()])
}

/// `launchpad netcap-probe [--quick] [--ca <file>] [--host <name>]`: checks run by a process under capture, as
/// a client that ignores proxy settings would see them; prints a JSON list of `{id, ok, detail}`.
pub fn probe(args: &[String]) -> ! {
    let quick = args.iter().any(|a| a == "--quick");
    let ca = args.iter().position(|a| a == "--ca").and_then(|i| args.get(i + 1)).cloned();
    let host = args.iter().position(|a| a == "--host").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "huggingface.co".into());
    let mut checks: Vec<serde_json::Value> = vec![];
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
    // The quick checks (the self-test) all pass; the full probe reports and the runner judges.
    let ok = !quick || checks.iter().all(|c| c["ok"] == true);
    println!("{}", serde_json::Value::Array(checks));
    std::process::exit(if ok { 0 } else { 1 })
}
