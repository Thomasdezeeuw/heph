//! Tests for the systemd integration.

use std::lazy::SyncLazy;
use std::pin::Pin;
use std::sync::{Mutex, MutexGuard};
use std::task::Poll;
use std::time::Duration;
use std::{env, panic, process};

use heph::{actor, rt, systemd, test};

const SOCKET_VAR: &'static str = "NOTIFY_SOCKET";
const WATCHDOG_PID_VAR: &'static str = "WATCHDOG_PID";
const WATCHDOG_TIMEOUT_VAR: &'static str = "WATCHDOG_USEC";

const VAR_LOCK: SyncLazy<Mutex<()>> = SyncLazy::new(|| Mutex::new(()));

fn with_var_lock<F, T>(f: F) -> T
where
    F: FnOnce() -> T,
{
    let var_lock = &*VAR_LOCK;
    let guard: MutexGuard<()> = var_lock.lock().unwrap();
    let res = panic::catch_unwind(panic::AssertUnwindSafe(f));
    drop(guard); // Don't poison the lock.
    match res {
        Ok(value) => value,
        Err(panic) => panic::resume_unwind(panic),
    }
}

#[test]
fn with_watchdog() {
    async fn actor<RT: rt::Access>(mut ctx: actor::Context<!, RT>) {
        let notify = with_var_lock(|| {
            env::set_var(SOCKET_VAR, "/tmp/abc");
            env::set_var(WATCHDOG_PID_VAR, process::id().to_string());
            env::set_var(WATCHDOG_TIMEOUT_VAR, "1000000");
            systemd::Notify::new(&mut ctx).unwrap().unwrap()
        });
        assert_eq!(notify.watchdog_timeout(), Some(Duration::from_secs(1)));
    }

    let (actor, _) = test::init_local_actor(actor as fn(_) -> _, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(
        test::poll_actor(Pin::as_mut(&mut actor)),
        Poll::Ready(Ok(()))
    );
}

#[test]
fn no_vars() {
    async fn actor<RT: rt::Access>(mut ctx: actor::Context<!, RT>) {
        with_var_lock(|| {
            env::remove_var(SOCKET_VAR);
            assert!(systemd::Notify::new(&mut ctx).unwrap().is_none());
        });
    }

    let (actor, _) = test::init_local_actor(actor as fn(_) -> _, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(
        test::poll_actor(Pin::as_mut(&mut actor)),
        Poll::Ready(Ok(()))
    );
}

#[test]
fn wrong_watchdog_pid() {
    async fn actor<RT: rt::Access>(mut ctx: actor::Context<!, RT>) {
        let notify = with_var_lock(|| {
            env::set_var(SOCKET_VAR, "/tmp/abc");
            env::set_var(WATCHDOG_PID_VAR, "1");
            env::set_var(WATCHDOG_TIMEOUT_VAR, "10000");
            systemd::Notify::new(&mut ctx).unwrap().unwrap()
        });
        // Wrong pid.
        assert_eq!(notify.watchdog_timeout(), None);
    }

    let (actor, _) = test::init_local_actor(actor as fn(_) -> _, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(
        test::poll_actor(Pin::as_mut(&mut actor)),
        Poll::Ready(Ok(()))
    );
}

#[test]
fn invalid_watchdog_pid() {
    async fn actor<RT: rt::Access>(mut ctx: actor::Context<!, RT>) {
        let notify = with_var_lock(|| {
            env::set_var(SOCKET_VAR, "/tmp/abc");
            env::set_var(WATCHDOG_PID_VAR, "NOT_A_NUMBER");
            env::set_var(WATCHDOG_TIMEOUT_VAR, "10000");
            systemd::Notify::new(&mut ctx).unwrap().unwrap()
        });
        // Invalid pid.
        assert_eq!(notify.watchdog_timeout(), None);
    }

    let (actor, _) = test::init_local_actor(actor as fn(_) -> _, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(
        test::poll_actor(Pin::as_mut(&mut actor)),
        Poll::Ready(Ok(()))
    );
}

#[test]
fn invalid_watchdog_timeout() {
    async fn actor<RT: rt::Access>(mut ctx: actor::Context<!, RT>) {
        let notify = with_var_lock(|| {
            env::set_var(SOCKET_VAR, "/tmp/abc");
            env::set_var(WATCHDOG_PID_VAR, process::id().to_string());
            env::set_var(WATCHDOG_TIMEOUT_VAR, "NOT_A_NUMBER");
            systemd::Notify::new(&mut ctx).unwrap().unwrap()
        });
        // Invalid timeout.
        assert_eq!(notify.watchdog_timeout(), None);
    }

    let (actor, _) = test::init_local_actor(actor as fn(_) -> _, ()).unwrap();
    let mut actor = Box::pin(actor);
    assert_eq!(
        test::poll_actor(Pin::as_mut(&mut actor)),
        Poll::Ready(Ok(()))
    );
}

/* TODO.


impl Notify {
    /// Create a systemd notifier connected to `path`.
    pub fn connect<M, RT, P>(
        ctx: &mut actor::Context<M, RT>,
        path: P,
        watchdog_timeout: Option<Duration>,
    ) -> io::Result<Notify>
    where
        RT: rt::Access,
        P: AsRef<Path>,
    {
        let mut socket = UnixDatagram::unbound()?;
        socket.connect(path)?;
        ctx.runtime().register(&mut socket, Interest::WRITABLE)?;
        #[cfg(target_os = "linux")]
        if let Some(cpu) = ctx.runtime_ref().cpu() {
            if let Err(err) = SockRef::from(&socket).set_cpu_affinity(cpu) {
                warn!("failed to set CPU affinity on systemd::Notify: {}", err);
            }
        }
        Ok(Notify {
            socket,
            watch_dog: watchdog_timeout,
        })
    }


    /// Inform the service manager of a change in the application state.
    ///
    /// `status` is a string to describe the service state. This is free-form
    /// and can be used for various purposes: general state feedback, fsck-like
    /// programs could pass completion percentages and failing programs could
    /// pass a human-readable error message. **Note that it must be limited to a
    /// single line.**
    pub fn change_state<'a>(&'a self, state: State, status: Option<&str>) -> ChangeState<'a> {
        let state_line = match state {
            State::Ready => "READY=1\n",
            State::Reloading => "RELOADING=1\n",
            State::Stopping => "STOPPING=1\n",
        };
        let state_update = match status {
            Some(status) => {
                let mut state_update =
                    String::with_capacity(state_line.len() + 7 + status.len() + 1);
                state_update.push_str(state_line);
                state_update.push_str("STATUS=");
                state_update.push_str(status);
                replace_newline(&mut state_update[state_line.len() + 7..]);
                state_update.push('\n');
                state_update
            }
            None => String::from(state_line),
        };
        ChangeState {
            notifier: self,
            state_update,
        }
    }

    /// Inform the service manager of a change in the application status.
    ///
    /// `status` is a string to describe the service state. This is free-form
    /// and can be used for various purposes: general state feedback, fsck-like
    /// programs could pass completion percentages and failing programs could
    /// pass a human-readable error message. **Note that it must be limited to a
    /// single line.**
    ///
    /// If you also need to change the state of the application you can use
    /// [`Notify::change_state`].
    pub fn change_status<'a>(&'a self, status: &str) -> ChangeState<'a> {
        let mut state_update = String::with_capacity(7 + status.len() + 1);
        state_update.push_str("STATUS=");
        state_update.push_str(status);
        replace_newline(&mut state_update[7..]);
        state_update.push('\n');
        ChangeState {
            notifier: self,
            state_update,
        }
    }

    /// Inform the service manager to update the watchdog timestamp.
    ///
    /// Send a keep-alive ping that services need to issue in regular intervals
    /// if `WatchdogSec=` is enabled for it.
    pub fn ping_watchdog<'a>(&'a self) -> PingWatchdog<'a> {
        PingWatchdog { notifier: self }
    }

    /// Inform the service manager that the service detected an internal error
    /// that should be handled by the configured watchdog options.
    ///
    /// This will trigger the same behaviour as if `WatchdogSec=` is enabled and
    /// the service did not call `ping_watchdog` in time.
    ///
    /// Note that `WatchdogSec=` does not need to be enabled for this to trigger
    /// the watchdog action. See [`systemd.service(5)`] for information about
    /// the watchdog behavior.
    ///
    /// [`systemd.service(5)`]: https://www.freedesktop.org/software/systemd/man/systemd.service.html
    pub fn trigger_watchdog<'a>(&'a self) -> TriggerWatchdog<'a> {
        TriggerWatchdog { notifier: self }
    }
}

/// Replaces new lines with spaces.
fn replace_newline(status: &mut str) {
    // SAFETY: replacing `\r` and `\n` with a space with is still valid UTF-8.
    for b in unsafe { status.as_bytes_mut().iter_mut() } {
        match *b {
            b'\r' | b'\n' => *b = b' ',
            _ => {}
        }
    }
}


impl<RT: rt::Access> actor::Bound<RT> for Notify {
    type Error = io::Error;

    fn bind_to<M>(&mut self, ctx: &mut actor::Context<M, RT>) -> io::Result<()> {
        ctx.runtime()
            .reregister(&mut self.socket, Interest::WRITABLE)
    }
}

/// Actor that manages the communication to the service manager.
///
/// It will set the application state (with the service manager) to ready when
/// it is spawned. Once it receives a signal (in the form of a  message) it will
/// set the state to stopping.
///
/// Finally it will ping the service manager if a watchdog is active. It will
/// check using `health_check` on the current status of the application.
#[allow(clippy::future_not_send)]
pub async fn actor<RT, H, E>(
    mut ctx: actor::Context<ServiceMessage, RT>,
    mut health_check: H,
) -> io::Result<()>
where
    RT: rt::Access + Clone,
    H: FnMut() -> Result<(), E>,
    E: ToString,
{
    let notify = match Notify::new(&mut ctx)? {
        Some(notify) => notify,
        None => {
            debug!("not started via systemd, not starting `systemd::actor`");
            return Ok(());
        }
    };
    notify.change_state(State::Ready, None).await?;

    if let Some(timeout) = notify.watchdog_timeout() {
        debug!("started via systemd with watchdog, timeout={:?}", timeout);
        let mut interval = Interval::every(&mut ctx, timeout);
        loop {
            match either(ctx.receive_next(), next(&mut interval)).await {
                Ok(Ok(msg)) => match msg {
                    ServiceMessage::ChangeState { state, status } => {
                        debug!(
                            "setting state to {:?}, {:?} with service manager",
                            state, status
                        );
                        notify.change_state(state, status.as_deref()).await?;
                        if let State::Stopping = state {
                            return Ok(());
                        }
                    }
                    ServiceMessage::ChangeStatus(status) => {
                        debug!("setting status with service manager to '{}'", status);
                        notify.change_status(&status).await?;
                    }
                },
                Ok(Err(_)) => {
                    // All actor references are dropped since we don't have any
                    // other stopping reason we'll stop now instead of running
                    // for ever.
                    warn!("all references to the systemd::watchdog are dropped, stopping it");
                    return Ok(());
                }
                // Deadline passed, ping the service manager.
                Err(_) => {
                    if let Err(err) = health_check() {
                        let err = err.to_string();
                        debug!("setting status with service manager to '{}'", err);
                        notify.change_status(&err).await?;
                    } else {
                        debug!("pinging service manager watchdog");
                        notify.ping_watchdog().await?;
                    }
                }
            }
        }
    } else {
        // No watchdog is active, so we'll wait for a stopping signal.
        while let Ok(msg) = ctx.receive_next().await {
            match msg {
                ServiceMessage::ChangeState { state, status } => {
                    debug!(
                        "setting state to {:?}, {:?} with service manager",
                        state, status
                    );
                    notify.change_state(state, status.as_deref()).await?;
                    if let State::Stopping = state {
                        return Ok(());
                    }
                }
                ServiceMessage::ChangeStatus(status) => {
                    debug!("setting status with service manager to '{}'", status);
                    notify.change_status(&status).await?;
                }
            }
        }
        debug!("setting state to stopping with service manager");
        notify.change_state(State::Stopping, None).await
    }
}

/// Message to send to the service manager.
#[non_exhaustive]
#[derive(Debug)]
pub enum ServiceMessage {
    /// Change the state of the application.
    ///
    /// See [`Notify::change_state`].
    ChangeState {
        /// The new state of the application.
        state: State,
        /// Description of the service state.
        status: Option<String>,
    },
    /// Describe the service state.
    ///
    /// See [`Notify::change_status`].
    ChangeStatus(String),
}

impl From<Terminate> for ServiceMessage {
    fn from(_: Terminate) -> ServiceMessage {
        ServiceMessage::ChangeState {
            state: State::Stopping,
            status: None,
        }
    }
}

impl TryFrom<Signal> for ServiceMessage {
    type Error = ();

    /// Converts [`Signal::Interrupt`], [`Signal::Terminate`] and
    /// [`Signal::Quit`], fails for all other signals (by returning `Err(())`).
    fn try_from(signal: Signal) -> Result<Self, Self::Error> {
        match signal {
            Signal::Interrupt | Signal::Terminate | Signal::Quit => {
                Ok(ServiceMessage::ChangeState {
                    state: State::Stopping,
                    status: None,
                })
            }
            _ => Err(()),
        }
    }
}
*/
