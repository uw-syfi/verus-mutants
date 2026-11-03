//! Runs oracle commands so that a timeout or an interrupt stops everything
//! they started, not only the direct child.
//!
//! An oracle is often a wrapper: a script that runs `cargo verus`, or a
//! `docker run` client whose container does the work. Killing only the direct
//! child would leave the rest running, still writing into the worker's target
//! directory while the next mutant builds there. On Unix each command
//! therefore runs in its own process group. On timeout the group gets
//! `SIGTERM` (which `docker run` forwards to its container), then `SIGKILL`
//! after a grace period. Because the groups no longer receive the terminal's
//! Ctrl-C, an interrupt handler terminates every live group before exiting.

use std::collections::BTreeSet;
use std::process::{Child, Command, ExitStatus};
use std::sync::{Mutex, Once};
use std::time::Duration;

use anyhow::{Context, Result};
use wait_timeout::ChildExt;

/// How long a timed-out command may take to exit after `SIGTERM`.
const GRACE: Duration = Duration::from_secs(10);

static LIVE_GROUPS: Mutex<BTreeSet<u32>> = Mutex::new(BTreeSet::new());
static HANDLER: Once = Once::new();

/// Exit status of a finished command, or `None` when it timed out and was
/// stopped.
pub struct Finished {
    pub status: Option<ExitStatus>,
}

/// Spawns `command` in its own process group and waits up to `timeout`.
pub fn run_with_timeout(command: &mut Command, timeout: Duration) -> Result<Finished> {
    install_interrupt_handler();
    isolate(command);
    let mut child = command.spawn()?;
    let group = child.id();
    LIVE_GROUPS
        .lock()
        .expect("live process groups")
        .insert(group);
    let result = wait(&mut child, group, timeout);
    LIVE_GROUPS
        .lock()
        .expect("live process groups")
        .remove(&group);
    result
}

fn wait(child: &mut Child, group: u32, timeout: Duration) -> Result<Finished> {
    if let Some(status) = child.wait_timeout(timeout)? {
        // Background processes the command left behind still share its group.
        signal_group(group, Signal::Kill);
        return Ok(Finished {
            status: Some(status),
        });
    }
    signal_group(group, Signal::Terminate);
    if child.wait_timeout(GRACE)?.is_none() {
        signal_group(group, Signal::Kill);
        child.kill().ok();
        child
            .wait()
            .context("waiting for a killed oracle command")?;
    }
    signal_group(group, Signal::Kill);
    Ok(Finished { status: None })
}

#[derive(Clone, Copy)]
enum Signal {
    Terminate,
    Kill,
}

#[cfg(unix)]
fn isolate(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn isolate(_command: &mut Command) {}

#[cfg(unix)]
fn signal_group(group: u32, signal: Signal) {
    let signal = match signal {
        Signal::Terminate => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    if let Ok(group) = libc::pid_t::try_from(group) {
        // SAFETY: `kill` takes no pointers; a negative pid names the process
        // group this module created with `process_group(0)`, whose id is the
        // child's pid. An already-empty group only yields ESRCH.
        unsafe {
            libc::kill(-group, signal);
        }
    }
}

#[cfg(not(unix))]
fn signal_group(_group: u32, _signal: Signal) {}

fn install_interrupt_handler() {
    HANDLER.call_once(|| {
        let installed = ctrlc::set_handler(|| {
            let groups = LIVE_GROUPS
                .lock()
                .map(|groups| groups.clone())
                .unwrap_or_default();
            for group in groups {
                signal_group(group, Signal::Terminate);
            }
            std::process::exit(130);
        });
        if let Err(error) = installed {
            eprintln!("warning: cannot install the interrupt handler: {error}");
        }
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::Path;

    /// A command whose own child outlives it must not survive the timeout.
    #[test]
    fn timeout_stops_the_whole_process_group() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("late");
        let script = format!(
            "sh -c 'sleep 3; touch \"$0\"' {} & sleep 30",
            marker.display()
        );
        let finished = run_with_timeout(
            Command::new("sh").args(["-c", &script]),
            Duration::from_millis(300),
        )
        .unwrap();
        assert!(finished.status.is_none());
        wait_until_gone(&marker);
    }

    /// Background processes left by a command that exited are stopped too.
    #[test]
    fn finished_command_leaves_no_background_processes() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("late");
        let script = format!("sh -c 'sleep 3; touch \"$0\"' {} &", marker.display());
        let finished = run_with_timeout(
            Command::new("sh").args(["-c", &script]),
            Duration::from_secs(30),
        )
        .unwrap();
        assert!(finished.status.is_some_and(|status| status.success()));
        wait_until_gone(&marker);
    }

    /// The marker would appear 3 s after start; checking past that proves the
    /// writer was stopped. The wait is a bound, not a synchronization.
    fn wait_until_gone(marker: &Path) {
        std::thread::sleep(Duration::from_secs(4));
        assert!(
            !marker.exists(),
            "a process of the group outlived the oracle"
        );
    }
}
