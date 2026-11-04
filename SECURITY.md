# Security model

CodeCurrency runs code that strangers wrote. That is the whole point, and it is
also the whole risk. This document states what the engine protects, how, and
what it does not protect. It describes the code in this repository, not a
deployment.

## What is trusted and what is not

Trusted:

- The CodeCurrency binary and its crate sources.
- The pinned toolchain images in `docker/` and the digests in
  `docker/toolchains.lock.json`.
- The host's Docker daemon.

Untrusted:

- Every submission: `main.c`, `main.cpp`, `main.s`, `main.rs`, `main.d`,
  `main.zig`, `main.odin`, `main.go`, and any extra source or build file in a
  solution directory.
- Anything the submission writes to stdout/stderr, to scratch space, or to a
  file it creates.
- The problem inputs a submission reads (they are engine-generated, but a
  submission may read them in unexpected ways).

The engine treats a submission as an adversary that wants to (a) escape the
sandbox, (b) read or write the host, (c) forge its own result, or (d) simply
exhaust the machine.

## The sandbox

Every run is one `docker run` with a fixed argument list built by
`ccx_core::sandbox::SandboxPolicy::argv`, which is a pure function of the policy
so it can be asserted on without a daemon. The controls, and the reason for
each:

| Control | Flag | Why |
| --- | --- | --- |
| No network | `--network=none` | A submission cannot phone home, exfiltrate input, or fetch a payload. |
| Non-root | `--user 65534:65534` | The submission is an unprivileged nobody inside the container. |
| Read-only root | `--read-only` | The image and every mounted source are immutable; there is nowhere to persist a change. |
| Scratch as tmpfs | `--tmpfs /work/scratch:rw,exec,nosuid,nodev` | The only writable place. `exec` is required because the compiled binary lives here; `nosuid`/`nodev` block privilege tricks. |
| `/tmp` noexec | `--tmpfs /tmp:rw,noexec,nosuid,nodev` | Removes a place to stage and run a second payload. |
| No capabilities | `--cap-drop=ALL` | No raw sockets, no `ptrace`-adjacent caps, no `chown`, no mounts. |
| No privilege gain | `--security-opt=no-new-privileges` | setuid/`setcap` binaries cannot elevate. |
| Memory cap | `--memory`, `--memory-swap` | A memory bomb is killed, not the host. |
| Process cap | `--pids-limit` | A fork bomb is contained. |
| CPU cap | `--cpus`, `--cpuset-cpus` | One core, pinned, so a submission cannot starve the host or migrate mid-measurement. |
| Host mounts read-only | `-v host:guest:ro` | Source and inputs are visible but not writable. |
| Hard limits | `ulimit -t -v -u -f -s` | Time, address space, processes, file size, stack. |
| Fixed hostname | `--hostname` | Determinism, not security: a random hostname perturbs the loader (see `AGENTS.md`). |
| Fixed environment | `--env` for every variable | Determinism: the measured environment is byte-identical everywhere. |

The default policy (`SandboxPolicy::default`) is 10 s wall time, 512 MiB
address space, 768 MiB container memory, 64 processes, 1 CPU, 1 MiB captured
output, 64 MiB scratch, 8 MiB stack.

## Result integrity

The engine, not the submission, decides the result.

- The runner emits its verdict as a `CCX-VERDICT {json}` line on its own
  stdout, which the engine parses into a typed struct
  (`ccx_core::engine::RunOutcome`). The submission's stdout/stderr are captured
  separately, so a submission cannot inject or edit the verdict line.
- Output is parsed only through the typed allow-listed fields of that struct;
  the engine never evaluates submission output as code.
- The `TamperWithResults` hostile case has a submission try to overwrite
  `/work/capsule.json`, `/work/metrics.json`, `/work/out/result.json`,
  `/work/.ccx-harness`, `/proc/sys/kernel/perf_event_paranoid`, and `/in/main.c`;
  the read-only root and unprivileged user keep all of them unchanged, and the
  suite asserts the host source file is byte-identical afterwards.
- The runner frames its report with length-prefixed sections (`CCX-OUT`,
  `CCX-ERR`, `CCX-ALLOC`, `CCX-EMIT`) and the verdict last, so the submission's
  own bytes can never be read as the verdict. The `OutputForgery` hostile case
  prints the framing sentinel itself and must not change the reported status.

## The hostile suite

`scripts/safety-suite.sh` builds and runs seventeen adversarial programs inside
the real pinned image and asserts containment by the *reason* each one stopped,
not merely that it stopped:

| Case | Expected containment |
| --- | --- |
| `infinite_loop` | Killed at the deadline (exit 124). |
| `fork_bomb` | Descendants reaped; process cap hit. |
| `huge_output` | Output cap hit (exit 125). |
| `huge_stderr` | Stderr capped like stdout (exit 125). |
| `memory_bomb` | Killed by the memory cap. |
| `deep_recursion` | Killed on stack overflow (SIGSEGV). |
| `nested_spawn` | A spawning tree is bounded by the pids limit. |
| `fd_exhaustion` | Descriptor exhaustion does not break the report. |
| `file_read_outside_sandbox` | An unmounted sentinel outside the mount set stays unreadable. |
| `symlink_escape` | A scratch symlink cannot reach a host path. |
| `proc_and_sys_probe` | No host bytes are readable through `/proc/1/root`, `/proc/kcore`, or `/proc/1/mem`. |
| `ptrace_attempt` | Attaching to another process is refused. |
| `signal_trick` | Signals from the child cannot kill or stop the runner. |
| `output_forgery` | The child's own framing cannot become the verdict. |
| `clock_tamper` | `clock_settime` and `settimeofday` are refused. |
| `network_attempt` | The connection does not succeed. |
| `tamper_with_results` | The host source is unchanged. |

The suite is part of `container-engine` CI, so a regression in containment fails
the build. The case sources live in `crates/ccx-core/src/sandbox.rs` and are
extracted by the suite, so the suite and the definition cannot drift apart.

## What this does not protect against

- **Docker/kernel escapes.** The sandbox is Docker isolation, not a VM. A kernel
  or runc escape defeats it. Phase 1 assumes a trusted, patched host daemon and
  is not multi-tenant hardened.
- **A malicious toolchain image.** The images are built by
  `scripts/pin-toolchains.sh` from pinned bases; if that build is compromised,
  the sandbox is compromised. Verify the digests in `docker/toolchains.lock.json`.
- **Denial of service across runs.** One submission cannot escape its own
  limits, but many submissions can still saturate the host's CPU and disk. There
  is no global scheduler in Phase 1.
- **Side channels.** Timing and cache side channels between concurrent runs on
  the same host are not mitigated.
- **`perf`/PMU access.** Hardware counters are opened by the runner via
  `perf_event_open` from inside the sandbox, and only when the host's
  `perf_event_paranoid` permits it (`<= 2`); the sandbox still drops all
  capabilities. A host that forbids PMU access simply reports the counters as
  unavailable. Enabling them does not relax any other containment control.

## Reporting a vulnerability

Open a private security advisory on the GitHub repository, or email the owner
listed in `Cargo.toml`. Do not open a public issue for an active vulnerability.
Include the capsule, the command, and the observed behaviour.
