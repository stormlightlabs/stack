//! Running a child process with a timeout and a cap on what it may write, so
//! a binary that hangs or floods its output cannot stall or exhaust `sltk`.
//! `sltk doctor` runs `<binary> --version` this way, and the GitHub host runs
//! `git` and `gh api`.

use std::io::{self, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// How often [`run_capped`] checks whether the process has exited.
const POLL: Duration = Duration::from_millis(10);

/// How long [`run_capped`] keeps reading once the process has exited. A
/// background process it started may hold a pipe open; reading stops after
/// this and the caller gets what was read.
const EXIT_GRACE: Duration = Duration::from_millis(200);

/// How many times [`run_capped`] starts a binary that the OS reports busy, as when an
/// install is still writing it.
const BUSY_ATTEMPTS: u32 = 5;

/// What a process wrote and how it exited, from [`run_capped`].
#[derive(Debug, Clone)]
pub struct Captured {
    /// How it exited.
    pub status: ExitStatus,
    /// Its stdout, cut at the cap.
    pub stdout: String,
    /// Its stderr, cut at the cap.
    pub stderr: String,
}

/// A pipe a reading thread fills, from [`Reader::start`].
struct Reader {
    /// What the thread has read so far.
    read: Arc<Mutex<Vec<u8>>>,
    /// Gets the thread's result once it stops.
    done: mpsc::Receiver<io::Result<()>>,
    /// The result, once received.
    result: Option<io::Result<()>>,
}

impl Reader {
    /// Starts a thread reading `from` up to `max` bytes.
    fn start(from: impl Read + Send + 'static, max: u64) -> Reader {
        let read = Arc::new(Mutex::new(Vec::new()));
        let (send, done) = mpsc::channel();
        let shared = Arc::clone(&read);
        std::thread::spawn(move || {
            let _ = send.send(read_capped(from, &shared, max));
        });
        Reader { read, done, result: None }
    }

    /// Waits for the thread until `deadline`, and says whether it stopped.
    fn wait_until(&mut self, deadline: Instant) -> bool {
        if self.result.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            self.result = self.done.recv_timeout(left).ok();
        }
        self.result.is_some()
    }

    /// What was read, with invalid UTF-8 replaced, or the read error.
    fn text(self) -> Result<String, String> {
        if let Some(Err(err)) = self.result {
            return Err(format!("could not read its output: {err}"));
        }
        let bytes = self.read.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Runs `command` with stdin closed and returns how it exited and what it
/// wrote, up to `max` bytes from each of stdout and stderr.
///
/// The process runs in a process group of its own. Once it exits, this
/// reads for up to [`EXIT_GRACE`] more and returns what it has, since a
/// background process it started may hold a pipe open. If it has not exited
/// after `timeout`, that is the error. Either way the whole group is then
/// killed. A process that left the group, as with `setsid`, can still hold a
/// pipe; its reading thread then waits on it until sltk exits.
pub fn run_capped(command: &mut Command, timeout: Duration, max: u64) -> Result<Captured, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(command, 0);
    let mut child = spawn_retrying(command).map_err(|err| format!("could not run it: {err}"))?;
    let mut stdout = Reader::start(child.stdout.take().expect("stdout is piped"), max);
    let mut stderr = Reader::start(child.stderr.take().expect("stderr is piped"), max);
    let exited = wait_for_exit(&mut child, Instant::now() + timeout);
    let grace = Instant::now() + if exited { EXIT_GRACE } else { Duration::ZERO };
    let finished = stdout.wait_until(grace) & stderr.wait_until(grace);
    kill_group(&mut child);
    let status = child.wait();
    if !exited {
        return Err(format!("it did not exit within {} s", timeout.as_secs_f32()));
    }
    if !finished {
        // The kill closes the pipes for every process in the group, so the
        // threads may now finish what is buffered.
        let grace = Instant::now() + EXIT_GRACE;
        stdout.wait_until(grace);
        stderr.wait_until(grace);
    }
    let status = status.map_err(|err| format!("could not wait for it: {err}"))?;
    Ok(Captured { status, stdout: stdout.text()?, stderr: stderr.text()? })
}

/// Reads `from` into `into` until end of file or `max` bytes, appending as
/// it goes so a caller that stops waiting still has what was read.
fn read_capped(mut from: impl Read, into: &Mutex<Vec<u8>>, max: u64) -> io::Result<()> {
    let mut buf = [0; 512];
    let mut total = 0;
    while total < max {
        let want = buf.len().min((max - total) as usize);
        let n = match from.read(&mut buf[..want]) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        into.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend_from_slice(&buf[..n]);
        total += n as u64;
    }
    Ok(())
}

/// Waits until `child` exits or `deadline` passes, and says whether it
/// exited. On Unix the child is left unreaped, so its process group ID cannot
/// be reused before [`kill_group`] signals it.
fn wait_for_exit(child: &mut std::process::Child, deadline: Instant) -> bool {
    loop {
        #[cfg(unix)]
        let status = {
            use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
            let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
            waitid(WaitId::Pid(Pid::from_child(child)), options).map(|status| status.is_some())
        };
        #[cfg(not(unix))]
        let status = child.try_wait().map(|status| status.is_some());
        match status {
            Ok(true) => return true,
            Ok(false) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => return false,
        }
    }
}

/// Kills every process in `child`'s process group on Unix, then `child`
/// itself, in case it moved to another group.
fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        use rustix::process::{Pid, Signal, kill_process_group};
        let _ = kill_process_group(Pid::from_child(child), Signal::KILL);
    }
    let _ = child.kill();
}

/// Starts `command`, retrying while the OS reports the file busy, as when an
/// install is still writing it.
fn spawn_retrying(command: &mut Command) -> io::Result<std::process::Child> {
    let mut attempt = 1;
    loop {
        match command.spawn() {
            Err(err) if err.kind() == io::ErrorKind::ExecutableFileBusy && attempt < BUSY_ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            result => return result,
        }
    }
}
