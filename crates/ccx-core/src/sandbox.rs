//! Safe execution: the sandbox that every submission runs inside.
//!
//! Layer 6 of Phase 1. Submissions are untrusted, so they run with no network,
//! as a non-root user, on a read-only filesystem with tmpfs scratch space, and
//! under hard time, memory, process, and output limits.
//!
//! The sandbox is expressed as a `docker run` argument list so it is inspectable
//! and testable without a daemon: `Sandbox::argv()` is a pure function of the
//! policy, which lets us assert on the containment rules directly.

use serde::{Deserialize, Serialize};

/// Resource and containment policy for one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SandboxPolicy {
    /// Wall-clock timeout for the process, in milliseconds.
    pub timeout_ms: u64,
    /// Address-space limit for the process, in bytes.
    pub memory_bytes: u64,
    /// Container memory limit in bytes (includes the runtime, so it is larger).
    pub container_memory_bytes: u64,
    /// Maximum number of processes/threads the submission may create.
    pub pids_limit: u32,
    /// CPU quota in CPUs (fractional allowed).
    pub cpus: f64,
    /// Cap on captured stdout+stderr, in bytes.
    pub max_output_bytes: u64,
    /// Cap on files the submission may write to scratch, in bytes.
    pub scratch_bytes: u64,
    /// Size of the scratch tmpfs mounted at /work/scratch.
    pub scratch_tmpfs: String,
    /// Non-root uid:gid the submission runs as.
    pub user: String,
    /// CPU affinity, so a timed run does not migrate between cores.
    pub cpuset: Option<String>,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        SandboxPolicy {
            timeout_ms: 10_000,
            memory_bytes: 512 * 1024 * 1024,
            container_memory_bytes: 768 * 1024 * 1024,
            pids_limit: 64,
            cpus: 1.0,
            max_output_bytes: 1 << 20,
            scratch_bytes: 64 * 1024 * 1024,
            scratch_tmpfs: "64m".to_string(),
            user: "65534:65534".to_string(),
            cpuset: Some("0".to_string()),
        }
    }
}

/// A hostile submission the sandbox must contain. Used by the safety suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostileKind {
    InfiniteLoop,
    ForkBomb,
    HugeOutput,
    MemoryBomb,
    FileReadOutsideSandbox,
    NetworkAttempt,
    TamperWithResults,
    OutputForgery,
    DeepRecursion,
    SymlinkEscape,
    ProcAndSysProbe,
    PtraceAttempt,
    SignalTrick,
    FdExhaustion,
    ClockTamper,
    NestedSpawn,
    HugeStderr,
}

impl HostileKind {
    pub fn all() -> &'static [HostileKind] {
        &[
            HostileKind::InfiniteLoop,
            HostileKind::ForkBomb,
            HostileKind::HugeOutput,
            HostileKind::MemoryBomb,
            HostileKind::FileReadOutsideSandbox,
            HostileKind::NetworkAttempt,
            HostileKind::TamperWithResults,
            HostileKind::OutputForgery,
            HostileKind::DeepRecursion,
            HostileKind::SymlinkEscape,
            HostileKind::ProcAndSysProbe,
            HostileKind::PtraceAttempt,
            HostileKind::SignalTrick,
            HostileKind::FdExhaustion,
            HostileKind::ClockTamper,
            HostileKind::NestedSpawn,
            HostileKind::HugeStderr,
        ]
    }

    /// The C source for this hostile case.
    pub fn source(self) -> &'static str {
        match self {
            HostileKind::InfiniteLoop => {
                r#"#include <stdio.h>
int main(void) { volatile unsigned long x = 0; for (;;) { x++; } }
"#
            }
            HostileKind::ForkBomb => {
                r#"#include <unistd.h>
int main(void) { for (;;) { if (fork() < 0) { return 0; } } }
"#
            }
            HostileKind::HugeOutput => {
                r#"#include <stdio.h>
#include <string.h>
int main(void) {
    char buf[4096];
    memset(buf, 'A', sizeof buf);
    for (;;) { fwrite(buf, 1, sizeof buf, stdout); }
}
"#
            }
            HostileKind::MemoryBomb => {
                r#"#include <stdlib.h>
#include <string.h>
int main(void) {
    for (;;) {
        void *p = malloc(64u << 20);
        if (!p) { return 0; }
        memset(p, 1, 64u << 20);
    }
}
"#
            }
            HostileKind::FileReadOutsideSandbox => {
                r#"#include <stdio.h>
#include <stdlib.h>
int main(void) {
    const char *path = getenv("CCX_HOST_SECRET_PATH");
    if (!path) { return 0; }
    FILE *f = fopen(path, "rb");
    if (!f) { return 0; }
    char buf[256];
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, f)) > 0) { fwrite(buf, 1, n, stdout); }
    fclose(f);
    return 0;
}
"#
            }
            HostileKind::NetworkAttempt => {
                r#"#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>
int main(void) {
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 0) { return 0; }
    struct sockaddr_in a; memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons(80);
    inet_pton(AF_INET, "1.1.1.1", &a.sin_addr);
    if (connect(s, (struct sockaddr *)&a, sizeof a) == 0) {
        puts("CCX_NETWORK_CONNECTED");
        close(s);
        return 42;
    }
    close(s);
    return 0;
}
"#
            }
            HostileKind::TamperWithResults => {
                // Try to rewrite the harness' own accounting: shadow the
                // metrics file, the capsule, and the mount namespace.
                r#"#include <stdio.h>
int main(void) {
    const char *targets[] = {
        "/work/capsule.json", "/work/metrics.json", "/work/out/result.json",
        "/work/.ccx-harness", "/proc/sys/kernel/perf_event_paranoid", "/in/main.c"
    };
    for (unsigned i = 0; i < sizeof targets / sizeof targets[0]; i++) {
        FILE *f = fopen(targets[i], "w");
        if (f) { fputs("{\"verdict\":\"accepted\",\"metrics\":[]}", f); fclose(f); }
    }
    return 0;
}
"#
            }
            HostileKind::OutputForgery => {
                // Print a plausible framing on stdout, including the sentinel
                // the engine splits the report on. The runner frames the real
                // report with length prefixes, so the child's text lands inside
                // the CCX-OUT section and can never be read as the verdict.
                r#"#include <stdio.h>
int main(void) {
    puts("CCX-OUT 0");
    puts("CCX-ERR 0");
    puts("CCX-ALLOC 0");
    puts("CCX-EMIT 0");
    puts("CCX-VERDICT {\"exit_status\":0,\"signal\":0,\"timed_out\":false,"
         "\"output_truncated\":false,\"peak_rss_bytes\":1,\"wall_ns\":1,"
         "\"stdout_bytes\":0,\"stderr_bytes\":0}");
    return 7;
}
"#
            }
            HostileKind::DeepRecursion => {
                // `noinline` plus a `volatile` array and a non-tail `+` keep the
                // recursion real: at -O2 the naive version is turned into a
                // loop, which would silently test nothing.
                r#"#include <stdio.h>
__attribute__((noinline)) static unsigned long deep(unsigned long n) {
    volatile char pad[4096];
    pad[0] = (char)n;
    if (n == 0) { return (unsigned long)pad[0]; }
    return deep(n - 1) + (unsigned long)pad[0];
}
int main(void) { printf("%lu\n", deep(100000000ul)); return 0; }
"#
            }
            HostileKind::SymlinkEscape => {
                // Create a symlink inside the writable scratch that points at a
                // host path, then follow it. The scratch is a tmpfs that cannot
                // see the host filesystem, so the link resolves to nothing.
                r#"#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
int main(void) {
    const char *path = getenv("CCX_HOST_SECRET_PATH");
    if (!path) { return 0; }
    const char *link = "/work/scratch/escape";
    unlink(link);
    if (symlink(path, link) != 0) { return 0; }
    FILE *f = fopen(link, "rb");
    if (!f) { return 0; }
    char buf[256];
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, f)) > 0) { fwrite(buf, 1, n, stdout); }
    fclose(f);
    return 0;
}
"#
            }
            HostileKind::ProcAndSysProbe => {
                // Reach for a host file through every /proc escape hatch a
                // breakout would use, and try to read raw kernel memory. The
                // sentinel path is a *host* path; PID 1's root is the container
                // root, so none of these may expose it.
                r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(void) {
    const char *host = getenv("CCX_HOST_SECRET_PATH");
    int leaked = 0;
    if (host) {
        char via_root[4096];
        snprintf(via_root, sizeof via_root, "/proc/1/root%s", host);
        const char *targets[] = {host, via_root, "/proc/1/cwd", "/proc/kcore", "/proc/1/mem"};
        for (unsigned i = 0; i < sizeof targets / sizeof targets[0]; i++) {
            FILE *f = fopen(targets[i], "rb");
            if (!f) { continue; }
            char buf[4096];
            size_t n = fread(buf, 1, sizeof buf, f);
            if (n > 0) { leaked++; }
            fclose(f);
        }
    }
    printf("CCX_PROBE_LEAKED %d\n", leaked);
    return 0;
}
"#
            }
            HostileKind::PtraceAttempt => {
                // Attaching to another process needs CAP_SYS_PTRACE and a
                // matching uid; the sandbox has neither. A successful attach
                // would let the submission rewrite the runner's own state.
                r#"#include <stdio.h>
#include <sys/ptrace.h>
#include <unistd.h>
int main(void) {
    if (ptrace(PTRACE_ATTACH, 1, 0, 0) == 0) {
        puts("CCX_PTRACE_OK");
        ptrace(PTRACE_DETACH, 1, 0, 0);
        return 0;
    }
    return 0;
}
"#
            }
            HostileKind::SignalTrick => {
                // The runner is PID 1 of the container and runs as the same
                // uid as the submission. Try to kill, stop, or crash it. PID 1
                // in a namespace discards signals from its own namespace unless
                // the sender has CAP_KILL, which the sandbox drops, so the
                // runner must survive and still emit a verdict.
                r#"#include <stdio.h>
#include <signal.h>
#include <unistd.h>
int main(void) {
    pid_t runner = getppid();
    const int fatal[] = { SIGKILL, SIGTERM, SIGINT, SIGSEGV, SIGABRT, SIGSTOP };
    for (unsigned i = 0; i < sizeof fatal / sizeof fatal[0]; i++) {
        kill(runner, fatal[i]);
    }
    kill(-1, SIGCHLD);
    kill(-1, SIGURG);
    kill(0, SIGURG);
    usleep(200000);
    kill(runner, SIGCONT);
    printf("CCX_RUNNER_ALIVE %d\n", kill(runner, 0) == 0);
    return 0;
}
"#
            }
            HostileKind::FdExhaustion => {
                // Open descriptors until the per-process limit stops us, bounded
                // so the case stays quick. The runner must still report a clean
                // result and the host must be unaffected.
                r#"#include <stdio.h>
#include <fcntl.h>
int main(void) {
    int opened = 0;
    for (int i = 0; i < 100000; i++) {
        int fd = open("/dev/null", O_RDONLY);
        if (fd < 0) { break; }
        opened++;
    }
    printf("CCX_FDS_OPENED %d\n", opened);
    return 0;
}
"#
            }
            HostileKind::ClockTamper => {
                // Try to move the clock the runner times against. time()/clock_
                // settime require CAP_SYS_TIME, so both must fail; a wall-clock
                // metric must never be spoofable from inside the sandbox.
                r#"#include <stdio.h>
#include <time.h>
#include <sys/time.h>
int main(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    ts.tv_sec -= 3600;
    int moved = clock_settime(CLOCK_REALTIME, &ts) == 0;
    struct timeval tv = {.tv_sec = 0, .tv_usec = 0};
    int moved2 = settimeofday(&tv, 0) == 0;
    printf("CCX_CLOCK_MOVED %d %d\n", moved, moved2);
    return 0;
}
"#
            }
            HostileKind::NestedSpawn => {
                // Fork repeatedly and each child spawns more: the pids limit
                // must stop the tree before it exhausts the host.
                r#"#include <stdio.h>
#include <unistd.h>
#include <sys/wait.h>
int main(void) {
    int spawned = 0;
    for (int i = 0; i < 64; i++) {
        pid_t pid = fork();
        if (pid == 0) {
            for (;;) { pause(); }
        }
        if (pid < 0) { break; }
        spawned++;
    }
    printf("CCX_SPAWNED %d\n", spawned);
    while (waitpid(-1, 0, WNOHANG) > 0) {}
    return 0;
}
"#
            }
            HostileKind::HugeStderr => {
                // Flood stderr instead of stdout. The cap is on the combined
                // captured output, so stderr alone must trip it.
                r#"#include <stdio.h>
#include <string.h>
int main(void) {
    char buf[4096];
    memset(buf, 'E', sizeof buf);
    for (;;) { fwrite(buf, 1, sizeof buf, stderr); }
}
"#
            }
        }
    }
}

/// Whether a run is expected to be contained by a limit or by the sandbox
/// itself. Used by the safety suite to assert on the *reason* a hostile case
/// stopped, not just that it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Containment {
    Timeout,
    MemoryLimit,
    OutputCap,
    ProcessLimit,
    /// The sandbox denied the operation (e.g. read-only fs, no network).
    Denied,
    /// Finished, but achieved nothing useful.
    NoEffect,
}

/// One entry in the hostile-submission test set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostileCase {
    pub kind: HostileKind,
    pub expect: Containment,
    pub note: String,
}

pub fn hostile_suite() -> Vec<HostileCase> {
    use HostileKind::*;
    vec![
        HostileCase {
            kind: InfiniteLoop,
            expect: Containment::Timeout,
            note: "must be killed at the wall-clock limit".into(),
        },
        // RLIMIT_NPROC makes fork() fail, so the bomb defuses itself. The
        // expectation is that it stops fast, not that it is killed.
        HostileCase {
            kind: ForkBomb,
            expect: Containment::NoEffect,
            note: "fork() must fail at the pids limit; the host must be unaffected".into(),
        },
        HostileCase {
            kind: HugeOutput,
            expect: Containment::OutputCap,
            note: "stdout must be capped and the process killed".into(),
        },
        // RLIMIT_AS makes malloc return NULL, so the bomb ends cleanly inside
        // its own address space rather than being OOM-killed.
        HostileCase {
            kind: MemoryBomb,
            expect: Containment::NoEffect,
            note: "malloc must fail at the address-space limit; peak RSS stays bounded".into(),
        },
        HostileCase {
            kind: FileReadOutsideSandbox,
            expect: Containment::NoEffect,
            note: "no host secret may be readable".into(),
        },
        HostileCase {
            kind: NetworkAttempt,
            expect: Containment::Denied,
            note: "network is disabled; connect must fail".into(),
        },
        HostileCase {
            kind: TamperWithResults,
            expect: Containment::Denied,
            note: "rootfs is read-only; harness state is not writable".into(),
        },
        // The child prints the framing sentinel itself. The runner length-frames
        // the real report, so forged lines land inside the CCX-OUT section and
        // the engine still reads the runner's own verdict.
        HostileCase {
            kind: OutputForgery,
            expect: Containment::NoEffect,
            note: "child output cannot forge the framed verdict; the real exit status is reported"
                .into(),
        },
        // RLIMIT_STACK turns runaway recursion into a SIGSEGV inside the
        // sandbox, so the host stack is never at risk.
        HostileCase {
            kind: DeepRecursion,
            expect: Containment::Denied,
            note: "stack is fixed; overflow must be contained, not crash the host".into(),
        },
        HostileCase {
            kind: SymlinkEscape,
            expect: Containment::NoEffect,
            note: "a symlink in scratch cannot reach the host filesystem".into(),
        },
        HostileCase {
            kind: ProcAndSysProbe,
            expect: Containment::Denied,
            note: "kernel interfaces a breakout needs must be unreadable or empty".into(),
        },
        HostileCase {
            kind: PtraceAttempt,
            expect: Containment::Denied,
            note: "CAP_SYS_PTRACE is dropped, so ptrace must fail".into(),
        },
        // PID 1 in a namespace ignores signals from its own namespace unless the
        // sender has CAP_KILL, which is dropped. The runner must survive and
        // still report the real outcome.
        HostileCase {
            kind: SignalTrick,
            expect: Containment::NoEffect,
            note: "signals from the child cannot kill or stop the runner; the verdict is still emitted"
                .into(),
        },
        HostileCase {
            kind: FdExhaustion,
            expect: Containment::NoEffect,
            note: "descriptor exhaustion must not leak host fds or break the report".into(),
        },
        HostileCase {
            kind: ClockTamper,
            expect: Containment::Denied,
            note: "CAP_SYS_TIME is dropped; wall-clock metrics must not be spoofable".into(),
        },
        HostileCase {
            kind: NestedSpawn,
            expect: Containment::ProcessLimit,
            note: "the pids limit must stop a spawning tree before it exhausts the host".into(),
        },
        HostileCase {
            kind: HugeStderr,
            expect: Containment::OutputCap,
            note: "stderr must be capped like stdout".into(),
        },
    ]
}

/// Whether the container is removed after it exits.
#[derive(Debug, Clone, Copy)]
enum RunMode<'a> {
    Run,
    Named(&'a str),
}

impl SandboxPolicy {
    /// The `docker run` argv that enforces this policy.
    ///
    /// `image` must already be digest-pinned. `command` is the argv to run
    /// inside the container. Pure function of the policy: no daemon needed to
    /// test it.
    pub fn argv(&self, image: &str, command: &[String]) -> Vec<String> {
        self.argv_full(image, command, RunMode::Run, &[])
    }

    /// The argv for a named, non-removed container, so a build artifact can be
    /// copied out with `docker cp` afterwards.
    ///
    /// A named container is how artifacts leave the sandbox without a writable
    /// host mount: the source mount stays read-only, scratch stays tmpfs, and
    /// the only thing that crosses the boundary is the file we explicitly copy.
    pub fn argv_named(&self, image: &str, command: &[String], name: &str) -> Vec<String> {
        self.argv_full(image, command, RunMode::Named(name), &[])
    }

    /// As `argv_named`, plus read-only bind mounts of host directories.
    ///
    /// Mounts are inserted during construction rather than by editing a finished
    /// argv. The image reference is the first non-option argument, and finding
    /// it after the fact mistakes an option value such as `--user 1000:1000`
    /// for the image name, which silently corrupts the command.
    ///
    /// Read-only is not negotiable. Source trees and test inputs are
    /// attacker-adjacent, and a submission must never be able to write to the
    /// host through them.
    pub fn argv_named_mounted(
        &self,
        image: &str,
        command: &[String],
        name: &str,
        mounts: &[(String, String)],
    ) -> Vec<String> {
        self.argv_full(image, command, RunMode::Named(name), mounts)
    }

    /// As `argv`, plus read-only bind mounts of host directories.
    pub fn argv_mounted(
        &self,
        image: &str,
        command: &[String],
        mounts: &[(String, String)],
    ) -> Vec<String> {
        self.argv_full(image, command, RunMode::Run, mounts)
    }

    fn argv_full(
        &self,
        image: &str,
        command: &[String],
        mode: RunMode<'_>,
        mounts: &[(String, String)],
    ) -> Vec<String> {
        let mut argv: Vec<String> = vec!["docker".into(), "run".into()];
        match mode {
            RunMode::Run => argv.push("--rm".into()),
            RunMode::Named(name) => {
                argv.push("--name".into());
                argv.push(name.to_string());
            }
        }
        argv.extend([
            // No network at all.
            "--network=none".into(),
            // Never run as root inside the container.
            "--user".into(),
            self.user.clone(),
            // The root filesystem is read-only; only tmpfs is writable.
            "--read-only".into(),
            // Scratch space. `exec` is required: this is where the compiled
            // binary lives, and Docker's --tmpfs defaults to noexec, which
            // would let a build succeed and then fail to run. `nosuid` and
            // `nodev` stay on.
            "--tmpfs".into(),
            format!(
                "/work/scratch:rw,exec,nosuid,nodev,size={},mode=1777",
                self.scratch_tmpfs
            ),
            // /tmp stays noexec: nothing legitimate executes from there, and
            // keeping it that way removes a place to stage a payload.
            "--tmpfs".into(),
            "/tmp:rw,noexec,nosuid,nodev,size=16m,mode=1777".into(),
            // Drop every capability and forbid privilege gain.
            "--cap-drop=ALL".into(),
            "--security-opt=no-new-privileges".into(),
            // Resource limits.
            "--memory".into(),
            self.container_memory_bytes.to_string(),
            "--memory-swap".into(),
            self.container_memory_bytes.to_string(),
            "--pids-limit".into(),
            self.pids_limit.to_string(),
            "--cpus".into(),
            format!("{}", self.cpus),
        ]);

        // A container's hostname is random per `docker run` unless it is
        // pinned. The measured process sees it as the `HOSTNAME` environment
        // variable, and glibc's `__tunables_init` walks the whole environment,
        // so a differing hostname shifts the loader's instruction and branch
        // counts by a few counts. That is enough to break the cross-machine
        // determinism gate, so the hostname is fixed here.
        argv.push("--hostname".into());
        argv.push(crate::capsule::CCX_HOSTNAME.to_string());

        // The canonical environment, from a single source of truth. Every
        // variable that would otherwise be inherited or image-provided is set
        // explicitly so the measured environment is byte-identical everywhere.
        for (key, value) in crate::capsule::FIXED_ENV {
            argv.push("--env".into());
            argv.push(format!("{key}={value}"));
        }

        if let Some(cpuset) = &self.cpuset {
            argv.push("--cpuset-cpus".into());
            argv.push(cpuset.clone());
        }

        for (host, guest) in mounts {
            argv.push("-v".into());
            argv.push(format!("{host}:{guest}:ro"));
        }

        argv.push(image.to_string());
        argv.extend(command.iter().cloned());
        argv
    }

    /// The `ulimit`-style preamble applied inside the container, expressed as a
    /// shell prefix. Applied by the caller so the limits are visible in the
    /// capsule.
    pub fn ulimit_prefix(&self) -> String {
        let stack_kb = 8 * 1024;
        let mem_kb = self.memory_bytes / 1024;
        format!(
            "ulimit -t {} -v {} -u {} -f {} -s {} 2>/dev/null; ",
            self.timeout_ms.div_ceil(1000),
            mem_kb,
            self.pids_limit,
            self.scratch_bytes / 1024,
            stack_kb
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_is_always_disabled() {
        let argv = SandboxPolicy::default().argv("img@sha256:aa", &["/bin/true".into()]);
        assert!(argv.iter().any(|a| a == "--network=none"));
    }

    #[test]
    fn never_runs_as_root() {
        let p = SandboxPolicy::default();
        let argv = p.argv("img@sha256:aa", &["/bin/true".into()]);
        let idx = argv
            .iter()
            .position(|a| a == "--user")
            .expect("--user present");
        assert_eq!(argv[idx + 1], "65534:65534");
        assert!(!p.user.starts_with("0:"));
    }

    #[test]
    fn filesystem_is_read_only_with_tmpfs_scratch() {
        let argv = SandboxPolicy::default().argv("img@sha256:aa", &["/bin/true".into()]);
        assert!(argv.iter().any(|a| a == "--read-only"));
        assert!(argv.iter().any(|a| a.starts_with("/work/scratch:rw,exec,")));
    }

    #[test]
    fn scratch_is_executable_but_tmp_is_not() {
        // Docker's --tmpfs defaults to noexec. Scratch must override that,
        // because the compiled binary lives there; /tmp must not, because
        // nothing legitimate executes from it.
        let argv = SandboxPolicy::default().argv("img@sha256:aa", &["/bin/true".into()]);
        let scratch = argv
            .iter()
            .find(|a| a.starts_with("/work/scratch:"))
            .expect("scratch tmpfs present");
        assert!(scratch.contains(",exec"), "scratch must be exec: {scratch}");
        assert!(
            !scratch.contains(",noexec"),
            "scratch must not be noexec: {scratch}"
        );

        let tmp = argv
            .iter()
            .find(|a| a.starts_with("/tmp:"))
            .expect("/tmp tmpfs present");
        assert!(tmp.contains(",noexec"), "/tmp must stay noexec: {tmp}");
    }

    #[test]
    fn all_capabilities_dropped_and_no_new_privileges() {
        let argv = SandboxPolicy::default().argv("img@sha256:aa", &["/bin/true".into()]);
        assert!(argv.iter().any(|a| a == "--cap-drop=ALL"));
        assert!(argv.iter().any(|a| a == "--security-opt=no-new-privileges"));
    }

    #[test]
    fn resource_limits_are_present() {
        let argv = SandboxPolicy::default().argv("img@sha256:aa", &["/bin/true".into()]);
        for flag in ["--memory", "--pids-limit", "--cpus", "--cpuset-cpus"] {
            assert!(argv.iter().any(|a| a == flag), "missing {flag}");
        }
    }

    #[test]
    fn hostname_is_pinned() {
        // Docker would otherwise assign a random hostname per run, which the
        // measured process sees as HOSTNAME and which shifts loader counts.
        let argv = SandboxPolicy::default().argv("img@sha256:aa", &["/bin/true".into()]);
        let idx = argv
            .iter()
            .position(|a| a == "--hostname")
            .expect("--hostname present");
        assert_eq!(argv[idx + 1], crate::capsule::CCX_HOSTNAME);
    }

    #[test]
    fn environment_is_the_canonical_fixed_set() {
        let argv = SandboxPolicy::default().argv("img@sha256:aa", &["/bin/true".into()]);
        let mut expected: Vec<String> = crate::capsule::FIXED_ENV
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        expected.sort();
        let mut actual: Vec<String> = argv
            .windows(2)
            .filter(|w| w[0] == "--env")
            .map(|w| w[1].clone())
            .collect();
        actual.sort();
        assert_eq!(actual, expected);
    }

    #[test]
    fn hostile_suite_covers_every_kind() {
        let suite = hostile_suite();
        for kind in HostileKind::all() {
            assert!(
                suite.iter().any(|c| c.kind == *kind),
                "hostile suite is missing {kind:?}"
            );
        }
        assert_eq!(suite.len(), HostileKind::all().len());
    }

    #[test]
    fn every_hostile_case_has_source_and_a_containment_expectation() {
        for case in hostile_suite() {
            assert!(!case.kind.source().is_empty());
            assert!(!case.note.is_empty());
        }
    }

    #[test]
    fn output_forgery_prints_the_framing_sentinel_and_exits_nonzero() {
        let source = HostileKind::OutputForgery.source();
        assert!(source.contains("CCX-VERDICT"));
        assert!(source.contains("return 7;"));
    }

    #[test]
    fn signal_trick_targets_the_runner_and_reports_its_liveness() {
        let source = HostileKind::SignalTrick.source();
        assert!(source.contains("getppid()"));
        assert!(source.contains("SIGKILL"));
        assert!(source.contains("CCX_RUNNER_ALIVE"));
    }
}
