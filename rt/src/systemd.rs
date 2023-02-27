//! Utilities to support [systemd].
//!
//! This module provides some utilities to run the application as a systemd
//! service, allowing it to inform systemd of it's state. This can only be done
//! if the service is started by systemd and it's `Type` set to `notify`, see
//! [`systemd.service(5)`] for more information.
//!
//! To notify systemd of changes you can use [`Notify`], which represents a
//! connection to the service manager. Or you can use the [`watchdog`] actor to
//! manage the communication with the service manager for you.
//!
//! [systemd]: https://systemd.io
//! [`systemd.service(5)`]: https://www.freedesktop.org/software/systemd/man/systemd.service.html#Type=

use std::convert::TryFrom;
use std::ffi::OsString;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::task::{self, Poll};
use std::time::Duration;
use std::{env, io, process};

use heph::actor;
use heph::messages::Terminate;
use log::{as_debug, debug, warn};
use mio::net::UnixDatagram;
use mio::Interest;
use socket2::SockRef;

use crate::timer::Interval;
use crate::util::{either, next};
use crate::{self as rt, Bound, Signal};

/// Systemd notifier.
///
/// This is only used by systemd if the service definition file has
/// `Type=notify` set, see [`systemd.service(5)`]. Read [`sd_notify(3)`] for
/// more information about notifying the service manager about start-up
/// completion and other service status changes.
///
/// [`systemd.service(5)`]: https://www.freedesktop.org/software/systemd/man/systemd.service.html#Type=
/// [`sd_notify(3)`]: https://www.freedesktop.org/software/systemd/man/sd_notify.html
#[derive(Debug)]
pub struct Notify {
    // TODO: replace with Heph version.
    socket: UnixDatagram,
    watch_dog: Option<Duration>,
}

impl Notify {
    /// Create a systemd notifier using the environment variables.
    ///
    /// This method uses the following environment variables to configure
    /// itself:
    /// * `NOTIFY_SOCKET`: the socket to connect to.
    /// * `WATCHDOG_PID` and `WATCHDOG_USEC`: enables the watchdog, see
    ///   [`systemd.service(5)`].
    ///
    /// Returns `None` if the environment `NOTIFY_SOCKET` variable is not set.
    ///
    /// [`systemd.service(5)`]: https://www.freedesktop.org/software/systemd/man/systemd.service.html#WatchdogSec=
    pub fn new<M, RT>(ctx: &mut actor::Context<M, RT>) -> io::Result<Option<Notify>>
    where
        RT: rt::Access,
    {
        const SOCKET_ENV_VAR: &str = "NOTIFY_SOCKET";
        const WATCHDOG_PID_ENV_VAR: &str = "WATCHDOG_PID";
        const WATCHDOG_USEC_ENV_VAR: &str = "WATCHDOG_USEC";

        let socket_path = env::var_os(SOCKET_ENV_VAR);
        let watchdog_pid = env::var_os(WATCHDOG_PID_ENV_VAR);
        let mut watchdog_timeout = env::var_os(WATCHDOG_USEC_ENV_VAR);

        if let Some(watchdog_pid) = watchdog_pid {
            match parse_os_string::<u32>(watchdog_pid) {
                // All good.
                Ok(pid) if pid == process::id() => {}
                // Either an invalid pid, or not meant for us.
                _ => watchdog_timeout = None,
            }
        }

        let mut notifier = match socket_path {
            Some(path) => Notify::connect(ctx, Path::new(&path))?,
            None => return Ok(None),
        };

        if let Some(watchdog_timeout) = watchdog_timeout {
            match parse_os_string(watchdog_timeout) {
                Ok(micros) => {
                    let timeout = Duration::from_micros(micros);
                    notifier.set_watchdog_timeout(Some(timeout));
                }
                Err(()) => {
                    warn!("{WATCHDOG_USEC_ENV_VAR} environment variable is invalid, ignoring it");
                }
            }
        }

        Ok(Some(notifier))
    }

    /// Create a systemd notifier connected to `path`.
    ///
    /// Also see [`Notify::new`] which creates a new `systemd::Notify` based on
    /// the environment variables set by systemd.
    pub fn connect<M, RT, P>(ctx: &mut actor::Context<M, RT>, path: P) -> io::Result<Notify>
    where
        RT: rt::Access,
        P: AsRef<Path>,
    {
        let mut socket = UnixDatagram::unbound()?;
        socket.connect(path)?;
        ctx.runtime().register(&mut socket, Interest::WRITABLE)?;
        if let Some(cpu) = ctx.runtime_ref().cpu() {
            if let Err(err) = SockRef::from(&socket).set_cpu_affinity(cpu) {
                warn!("failed to set CPU affinity on systemd::Notify: {err}");
            }
        }
        Ok(Notify {
            socket,
            watch_dog: None,
        })
    }

    /// Set the watchdog timeout of `Notify`.
    ///
    /// Note that this doesn't change the timeout for the service manager.
    pub fn set_watchdog_timeout(&mut self, timeout: Option<Duration>) {
        self.watch_dog = timeout;
    }

    /// Returns the watchdog timeout, if any.
    pub fn watchdog_timeout(&self) -> Option<Duration> {
        self.watch_dog
    }

    /// Inform the service manager of a change in the application state.
    ///
    /// `status` is a string to describe the service state. This is free-form
    /// and can be used for various purposes: general state feedback, fsck-like
    /// programs could pass completion percentages and failing programs could
    /// pass a human-readable error message. **Note that it must be limited to a
    /// single line.**
    pub fn change_state<'a>(&'a self, state: State, status: Option<&str>) -> ChangeState<'a> {
        debug!(state = log::as_debug!(state), status = log::as_debug!(status); "updating state with service manager");
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
        debug!(status = log::as_display!(status); "updating status with service manager");
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
        debug!("pinging service manager watchdog");
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
        debug!("triggering service manager watchdog");
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

fn parse_os_string<T: FromStr>(str: OsString) -> Result<T, ()> {
    match str.into_string() {
        Ok(str) => match str.parse() {
            Ok(value) => Ok(value),
            Err(_) => Err(()),
        },
        Err(_) => Err(()),
    }
}

/// State of the application.
#[non_exhaustive]
#[derive(Copy, Clone, Debug)]
pub enum State {
    /// Indicate the service startup is finished, or the service finished
    /// loading its configuration.
    Ready,
    /// Indicate the service is reloading its configuration.
    ///
    /// This is useful to allow the service manager to track the service's
    /// internal state, and present it to the user.
    ///
    /// Note that a service that sends this notification must also send a
    /// [`Ready`] notification when it completed reloading its configuration.
    /// Reloads are propagated in the same way as they are when initiated by the
    /// user.
    ///
    /// [`Ready`]: State::Ready
    Reloading,
    /// Indicate the service is beginning its shutdown.
    ///
    /// This is useful to allow the service manager to track the service's
    /// internal state, and present it to the user.
    Stopping,
}

/// The [`Future`] behind [`Notify::change_state`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct ChangeState<'a> {
    notifier: &'a Notify,
    state_update: String,
}

impl<'a> Future for ChangeState<'a> {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        try_io!(self.notifier.socket.send(self.state_update.as_bytes())).map_ok(|_| ())
    }
}

/// The [`Future`] behind [`Notify::ping_watchdog`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct PingWatchdog<'a> {
    notifier: &'a Notify,
}

impl<'a> Future for PingWatchdog<'a> {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        try_io!(self.notifier.socket.send(b"WATCHDOG=1")).map_ok(|_| ())
    }
}

/// The [`Future`] behind [`Notify::trigger_watchdog`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct TriggerWatchdog<'a> {
    notifier: &'a Notify,
}

impl<'a> Future for TriggerWatchdog<'a> {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        try_io!(self.notifier.socket.send(b"WATCHDOG=trigger")).map_ok(|_| ())
    }
}

impl<RT: rt::Access> Bound<RT> for Notify {
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
/// update the state accordingly.
///
/// Finally it will ping the service manager if a watchdog is active. It will
/// check using `health_check` on the current status of the application.
///
/// # Notes
///
/// This actor will stop if one of the following conditions is true:
///  * If the service is not started by systemd.
///  * Once this receives a message with state set to `State::Stopping`.
///  * Once all actor references pointing to this actor are dropped. If this
///    happens it will also set the state to `State::Stopping`.
///
/// If no systemd watchdog is active (i.e. `WatchdogSec` is not set in the
/// systemd service configuration) this will not call `health_check`.
#[allow(clippy::future_not_send)]
pub async fn watchdog<RT, H, E>(
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
            debug!("not started via systemd, not starting `systemd::watchdog`");
            return Ok(());
        }
    };
    notify.change_state(State::Ready, None).await?;

    if let Some(timeout) = notify.watchdog_timeout() {
        debug!(timeout = as_debug!(timeout); "started via systemd with watchdog");
        let mut interval = Interval::every(&mut ctx, timeout);
        loop {
            match either(ctx.receive_next(), next(&mut interval)).await {
                Ok(Ok(msg)) => match msg {
                    ServiceMessage::ChangeState { state, status } => {
                        notify.change_state(state, status.as_deref()).await?;
                        if let State::Stopping = state {
                            return Ok(());
                        }
                    }
                    ServiceMessage::ChangeStatus(status) => notify.change_status(&status).await?,
                },
                Ok(Err(_)) => {
                    // All actor references are dropped since we don't have any
                    // other stopping reason we'll stop now instead of running
                    // for ever.
                    break;
                }
                // Deadline passed, ping the service manager.
                Err(_) => {
                    if let Err(err) = health_check() {
                        let err = err.to_string();
                        notify.change_status(&err).await?;
                    } else {
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
                    notify.change_state(state, status.as_deref()).await?;
                    if let State::Stopping = state {
                        return Ok(());
                    }
                }
                ServiceMessage::ChangeStatus(status) => {
                    notify.change_status(&status).await?;
                }
            }
        }
    }
    debug!("all references to the systemd::watchdog are dropped, stopping it");
    notify.change_state(State::Stopping, None).await
}

/// Message to send to the service manager.
///
/// # Notes
///
/// The implements [`TryFrom`]`<`[`Signal`]`>` so that the actor reference can
/// handle process signal automatically by setting the state to
/// [`State::Stopping`].
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
                    status: Some("stopping after process signal".into()),
                })
            }
            _ => Err(()),
        }
    }
}
