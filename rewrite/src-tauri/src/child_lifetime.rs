//! Native child lifetime on Linux, the counterpart of the Windows job objects
//! with `KILL_ON_JOB_CLOSE`. Each contained child gets its own process group
//! and `PR_SET_PDEATHSIG(SIGKILL)`, so a crash or `kill -9` of SonoJot cannot
//! leave a worker or llama-server holding RAM and VRAM.
//!
//! The death signal follows the *thread* that forked the child, not the
//! process. Spawns usually run on async-runtime blocking threads, which exit
//! when idle, so every contained child is forked by one thread that lives as
//! long as the process. Other platforms spawn directly.
use std::{io, process::{Child, Command}};

#[cfg(target_os = "linux")]
pub(crate) fn spawn(command: &mut Command) -> io::Result<Child> {
    use std::os::unix::process::CommandExt;
    use std::sync::{mpsc, Mutex, OnceLock};

    struct Request(*mut Command, mpsc::SyncSender<io::Result<Child>>);
    // The caller blocks until the reply arrives, so the borrowed Command
    // outlives its use on the spawner thread and is never shared.
    unsafe impl Send for Request {}

    static SPAWNER: OnceLock<Mutex<mpsc::Sender<Request>>> = OnceLock::new();
    let sender = SPAWNER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Request>();
        let _ = std::thread::Builder::new()
            .name("child-spawner".into())
            .spawn(move || {
                for Request(command, reply) in receiver {
                    let _ = reply.send(unsafe { &mut *command }.spawn());
                }
            });
        Mutex::new(sender)
    });
    let parent = std::process::id() as libc::pid_t;
    unsafe {
        command.pre_exec(move || {
            if libc::setpgid(0, 0) != 0
                || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) != 0
            {
                return Err(io::Error::last_os_error());
            }
            // The parent may have exited before prctl took effect.
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
    let (reply, result) = mpsc::sync_channel(1);
    sender
        .lock()
        .map_err(|_| io::Error::other("child spawner is unavailable"))?
        .send(Request(command as *mut Command, reply))
        .map_err(|_| io::Error::other("child spawner is unavailable"))?;
    result
        .recv()
        .map_err(|_| io::Error::other("child spawner is unavailable"))?
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn spawn(command: &mut Command) -> io::Result<Child> {
    command.spawn()
}

/// Kill the child and anything it started in its process group, then the
/// child itself. Only signals the group while the leader is unreaped, so a
/// recycled PID can never target an unrelated process group.
pub(crate) fn kill(child: &mut Child) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    if matches!(child.try_wait(), Ok(None)) {
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
    }
    child.kill()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn contained_child_has_its_own_group_and_survives_the_calling_thread() {
        let mut child = std::thread::spawn(|| spawn(Command::new("sleep").arg("30")).unwrap())
            .join()
            .unwrap();
        // The thread that asked for the spawn has exited; the child must not
        // have received its death signal.
        std::thread::sleep(Duration::from_millis(200));
        assert!(matches!(child.try_wait(), Ok(None)));
        let group = unsafe { libc::getpgid(child.id() as libc::pid_t) };
        assert_eq!(group, child.id() as libc::pid_t);
        kill(&mut child).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(child.try_wait().unwrap().is_some());
    }
}
