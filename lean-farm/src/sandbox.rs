//! Real, sandboxed Lean execution.
//!
//! [`Sandbox`] runs a self-contained Lean source file inside a hardened
//! container and reports a structured [`SandboxOutcome`] (exit status, captured
//! stdout/stderr, wall-clock duration, timeout flag). This is the proving hot
//! path: a proof is considered checked if and only if the Lean process exits
//! successfully after elaborating the source.
//!
//! # Security posture
//!
//! Every container is launched with defence-in-depth flags derived from the
//! operator configuration:
//! - `--network=none` (default) — no egress; stdlib proofs need no network.
//! - `--read-only` root filesystem with a size-capped `tmpfs` for `/tmp`.
//! - `--user <uid>:<gid>` — non-root execution.
//! - `--cap-drop=ALL` and `--security-opt=no-new-privileges` — no privilege
//!   escalation and no ambient capabilities.
//! - `--cpus`, `--memory`, `--memory-swap`, `--pids-limit` — resource ceilings.
//! - The proof source is bind-mounted **read-only**.
//! - Optional `--runtime=runsc` for gVisor syscall isolation.
//!
//! # Determinism & testability
//!
//! Argument construction is a pure function ([`Sandbox::build_run_args`]) so the
//! exact hardening flags can be asserted in unit tests without Docker. The
//! actual container execution is exercised by the Docker-gated E2E test suite.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::config::{Config, SandboxRuntime};
use crate::{BoxError, LeanFarmError};

/// The container command used to check a proof source. The stock
/// `leanprover/lean4` image exposes `lean` on `PATH`; elaborating a file
/// returns a non-zero exit code if and only if there is an error (including an
/// unproven goal), which is exactly the success signal we want.
const LEAN_BINARY: &str = "lean";

/// Fixed name of the proof source file written into the sandbox workdir.
const PROOF_FILE_NAME: &str = "Proof.lean";

/// Structured result of a single sandboxed Lean run.
#[derive(Debug, Clone)]
pub struct SandboxOutcome {
    /// True when Lean elaborated the source without error (exit code 0).
    pub success: bool,
    /// Process exit code, when the process terminated normally.
    pub exit_code: Option<i32>,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error (Lean diagnostics land here).
    pub stderr: String,
    /// Wall-clock duration of the run.
    pub duration: Duration,
    /// True when the run was killed for exceeding its time budget.
    pub timed_out: bool,
}

impl SandboxOutcome {
    /// A concise, human-readable error message describing why a run failed.
    /// Returns `None` for a successful run.
    pub fn failure_message(&self) -> Option<String> {
        if self.success {
            return None;
        }
        if self.timed_out {
            return Some(format!(
                "lean execution timed out after {:.1}s",
                self.duration.as_secs_f64()
            ));
        }
        let detail = if !self.stderr.trim().is_empty() {
            self.stderr.trim()
        } else if !self.stdout.trim().is_empty() {
            self.stdout.trim()
        } else {
            "no diagnostics captured"
        };
        let code = self
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        Some(format!("lean exited with status {code}: {detail}"))
    }
}

/// Hardened Lean container runner.
#[derive(Clone, Debug)]
pub struct Sandbox {
    docker_binary: String,
    runtime: SandboxRuntime,
    image: String,
    network: String,
    workdir: String,
    pids_limit: u32,
    tmpfs_size_mb: u64,
    read_only_root: bool,
    cpus: f64,
    memory_bytes: u64,
    run_as_user: u32,
    run_as_group: u32,
    extra_run_args: Vec<String>,
}

impl Sandbox {
    /// Build a sandbox from the full configuration tree. Resource ceilings and
    /// the non-root identity are taken from the `security` section so the
    /// sandbox and the operator's declared security posture cannot drift apart.
    pub fn from_config(config: &Config) -> Self {
        let docker_binary =
            std::env::var("LEAN_FARM_DOCKER_BIN").unwrap_or_else(|_| "docker".to_string());
        Self {
            docker_binary,
            runtime: config.sandbox.runtime,
            image: config.lean.image.clone(),
            network: config.sandbox.network.clone(),
            workdir: config.sandbox.workdir.clone(),
            pids_limit: config.sandbox.pids_limit,
            tmpfs_size_mb: config.sandbox.tmpfs_size_mb,
            read_only_root: config.sandbox.read_only_root,
            cpus: config.security.resource_limits.cpu_limit,
            memory_bytes: config.security.resource_limits.memory_limit,
            run_as_user: config.security.run_as_user,
            run_as_group: config.security.run_as_group,
            extra_run_args: config.sandbox.extra_run_args.clone(),
        }
    }

    /// Return `Ok(())` when the container runtime is reachable, otherwise a
    /// [`LeanFarmError::NotConfigured`] describing what is missing. Callers use
    /// this to distinguish "infrastructure absent" from "proof failed".
    pub async fn ensure_available(&self) -> Result<(), BoxError> {
        let output = Command::new(&self.docker_binary)
            .args(["version", "--format", "{{.Server.Version}}"])
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => {
                let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
                debug!("container runtime available (server {version})");
                Ok(())
            }
            Ok(out) => Err(LeanFarmError::NotConfigured(format!(
                "container runtime '{}' is installed but not ready: {}",
                self.docker_binary,
                String::from_utf8_lossy(&out.stderr).trim()
            ))
            .into()),
            Err(e) => Err(LeanFarmError::NotConfigured(format!(
                "container runtime '{}' is not available: {e}",
                self.docker_binary
            ))
            .into()),
        }
    }

    /// Construct the full `docker run` argument vector for a proof run.
    ///
    /// Pure and side-effect free so the hardening flags can be asserted in unit
    /// tests. `host_dir` is the host directory (bind-mounted read-only) that
    /// contains [`PROOF_FILE_NAME`]; `container_name` is used so the run can be
    /// force-killed on timeout.
    pub fn build_run_args(&self, host_dir: &Path, container_name: &str) -> Vec<String> {
        let host_dir = host_dir.to_string_lossy().to_string();
        let proof_path = format!("{}/{}", self.workdir.trim_end_matches('/'), PROOF_FILE_NAME);
        let mut args: Vec<String> = vec!["run".into(), "--rm".into()];

        args.push(format!("--name={container_name}"));

        if matches!(self.runtime, SandboxRuntime::Gvisor) {
            args.push("--runtime=runsc".into());
        }

        // Isolation.
        args.push(format!("--network={}", self.network));
        args.push("--cap-drop=ALL".into());
        args.push("--security-opt=no-new-privileges".into());
        args.push(format!("--user={}:{}", self.run_as_user, self.run_as_group));

        // Filesystem: read-only root plus a size-capped writable tmpfs used for
        // /tmp and as HOME. `noexec,nosuid,nodev` prevent code execution and
        // privilege tricks from the scratch space.
        if self.read_only_root {
            args.push("--read-only".into());
        }
        args.push(format!(
            "--tmpfs=/tmp:rw,noexec,nosuid,nodev,size={}m",
            self.tmpfs_size_mb
        ));
        args.push("--env=HOME=/tmp".into());

        // Resource ceilings. memory-swap == memory disables swap usage.
        if self.cpus > 0.0 {
            args.push(format!("--cpus={}", self.cpus));
        }
        if self.memory_bytes > 0 {
            args.push(format!("--memory={}", self.memory_bytes));
            args.push(format!("--memory-swap={}", self.memory_bytes));
        }
        args.push(format!("--pids-limit={}", self.pids_limit));

        // Read-only bind mount of the proof source and a fixed workdir.
        args.push(format!("--volume={host_dir}:{}:ro", self.workdir));
        args.push(format!("--workdir={}", self.workdir));

        for extra in &self.extra_run_args {
            args.push(extra.clone());
        }

        // Image and the command to run.
        args.push(self.image.clone());
        args.push(LEAN_BINARY.into());
        args.push(proof_path);

        args
    }

    /// Write `source` to a fresh temp directory, run it through the sandbox with
    /// the given time budget, and return the structured outcome. The temp
    /// directory is always removed before returning.
    pub async fn run_lean_source(
        &self,
        source: &str,
        time_budget: Duration,
    ) -> Result<SandboxOutcome, BoxError> {
        let workspace = TempWorkspace::create().await?;
        let proof_file = workspace.dir.join(PROOF_FILE_NAME);
        tokio::fs::write(&proof_file, source).await.map_err(|e| {
            LeanFarmError::JobExecution(format!(
                "failed to stage proof source at {}: {e}",
                proof_file.display()
            ))
        })?;

        let container_name = format!("lean-farm-{}", uuid::Uuid::new_v4());
        let args = self.build_run_args(&workspace.dir, &container_name);

        info!(
            "running lean sandbox (runtime={:?}, image={}, budget={:.0}s)",
            self.runtime,
            self.image,
            time_budget.as_secs_f64()
        );
        debug!("{} {}", self.docker_binary, args.join(" "));

        let start = Instant::now();
        let mut command = Command::new(&self.docker_binary);
        command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let child = command.spawn().map_err(|e| {
            LeanFarmError::JobExecution(format!("failed to spawn '{}': {e}", self.docker_binary))
        })?;

        match timeout(time_budget, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let duration = start.elapsed();
                let success = output.status.success();
                let outcome = SandboxOutcome {
                    success,
                    exit_code: output.status.code(),
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                    duration,
                    timed_out: false,
                };
                if success {
                    info!("lean sandbox succeeded in {:.2}s", duration.as_secs_f64());
                } else {
                    warn!(
                        "lean sandbox failed (exit={:?}) in {:.2}s",
                        outcome.exit_code,
                        duration.as_secs_f64()
                    );
                }
                Ok(outcome)
            }
            Ok(Err(e)) => {
                Err(LeanFarmError::JobExecution(format!("error awaiting lean sandbox: {e}")).into())
            }
            Err(_elapsed) => {
                // Force-remove the container by name; `--rm` cleans up the
                // stopped container. The dropped child future kills the client.
                warn!(
                    "lean sandbox exceeded {:.0}s budget; killing container {container_name}",
                    time_budget.as_secs_f64()
                );
                let _ = Command::new(&self.docker_binary)
                    .args(["kill", &container_name])
                    .output()
                    .await;
                Ok(SandboxOutcome {
                    success: false,
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    duration: start.elapsed(),
                    timed_out: true,
                })
            }
        }
    }
}

/// RAII temp directory that is best-effort removed on drop, with an explicit
/// async cleanup performed at the end of a run.
struct TempWorkspace {
    dir: PathBuf,
}

impl TempWorkspace {
    async fn create() -> Result<Self, BoxError> {
        let dir = std::env::temp_dir().join(format!("lean-farm-work-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.map_err(|e| {
            LeanFarmError::JobExecution(format!(
                "failed to create sandbox workspace {}: {e}",
                dir.display()
            ))
        })?;
        Ok(Self { dir })
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        // Best-effort synchronous cleanup; the directory only ever holds a
        // single small text file, so this is cheap and cannot block meaningfully.
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn sandbox() -> Sandbox {
        Sandbox::from_config(&Config::default())
    }

    #[test]
    fn run_args_are_hardened_by_default() {
        let sb = sandbox();
        let dir = Path::new("/host/work");
        let args = sb.build_run_args(dir, "lean-farm-test");

        // Core invocation.
        assert_eq!(args[0], "run");
        assert!(args.contains(&"--rm".to_string()));
        assert!(args.contains(&"--name=lean-farm-test".to_string()));

        // Isolation flags.
        assert!(args.contains(&"--network=none".to_string()));
        assert!(args.contains(&"--cap-drop=ALL".to_string()));
        assert!(args.contains(&"--security-opt=no-new-privileges".to_string()));
        assert!(args.contains(&"--user=1000:1000".to_string()));
        assert!(args.contains(&"--read-only".to_string()));

        // Resource ceilings present.
        assert!(args.iter().any(|a| a.starts_with("--cpus=")));
        assert!(args.iter().any(|a| a.starts_with("--memory=")));
        assert!(args.iter().any(|a| a.starts_with("--memory-swap=")));
        assert!(args.iter().any(|a| a.starts_with("--pids-limit=")));

        // Read-only bind mount and workdir.
        assert!(args
            .iter()
            .any(|a| a.contains("/host/work:/work:ro") || a.contains("\\host\\work:/work:ro")));
        assert!(args.contains(&"--workdir=/work".to_string()));

        // Command runs lean on the staged proof file, last two args.
        let n = args.len();
        assert_eq!(args[n - 2], "lean");
        assert_eq!(args[n - 1], "/work/Proof.lean");
    }

    #[test]
    fn gvisor_runtime_adds_runsc_flag() {
        let mut config = Config::default();
        config.sandbox.runtime = SandboxRuntime::Gvisor;
        let sb = Sandbox::from_config(&config);
        let args = sb.build_run_args(Path::new("/tmp/x"), "n");
        assert!(args.contains(&"--runtime=runsc".to_string()));
    }

    #[test]
    fn network_mode_is_configurable() {
        let mut config = Config::default();
        config.sandbox.network = "bridge".to_string();
        let sb = Sandbox::from_config(&config);
        let args = sb.build_run_args(Path::new("/tmp/x"), "n");
        assert!(args.contains(&"--network=bridge".to_string()));
        assert!(!args.contains(&"--network=none".to_string()));
    }

    #[test]
    fn failure_message_prefers_stderr() {
        let outcome = SandboxOutcome {
            success: false,
            exit_code: Some(1),
            stdout: String::new(),
            stderr: "Proof.lean:3:0: error: unsolved goals".to_string(),
            duration: Duration::from_millis(10),
            timed_out: false,
        };
        let msg = outcome.failure_message().unwrap();
        assert!(msg.contains("status 1"));
        assert!(msg.contains("unsolved goals"));
    }

    #[test]
    fn failure_message_reports_timeout() {
        let outcome = SandboxOutcome {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration: Duration::from_secs(30),
            timed_out: true,
        };
        assert!(outcome.failure_message().unwrap().contains("timed out"));
    }

    #[test]
    fn successful_outcome_has_no_failure_message() {
        let outcome = SandboxOutcome {
            success: true,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            duration: Duration::from_millis(1),
            timed_out: false,
        };
        assert!(outcome.failure_message().is_none());
    }
}
