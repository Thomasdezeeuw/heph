use std::fmt;

/// Process signal.
///
/// Actors can receive signals by calling [`Runtime::receive_signals`] or
/// [`RuntimeRef::receive_signals`] with their actor reference.
///
/// Synchronous actors receive signals automatically if they implement the
/// required trait bounds, see [`SyncActor`] for more details.
///
/// [`Runtime::receive_signals`]: crate::Runtime::receive_signals
/// [`RuntimeRef::receive_signals`]: crate::RuntimeRef::receive_signals
/// [`SyncActor`]: crate::actor::sync::SyncActor
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Signal {
    /// Interrupt signal.
    ///
    /// This signal is received by the process when its controlling terminal
    /// wishes to interrupt the process. This signal will for example be send
    /// when Ctrl+C is pressed in most terminals.
    ///
    /// Corresponds to POSIX signal `SIGINT`.
    Interrupt,
    /// Termination request signal.
    ///
    /// This signal received when the process is requested to terminate. This
    /// allows the process to perform nice termination, releasing resources and
    /// saving state if appropriate. This signal will be send when using the
    /// `kill` command for example.
    ///
    /// Corresponds to POSIX signal `SIGTERM`.
    Terminate,
    /// Terminal quit signal.
    ///
    /// This signal is received when the process is requested to quit and
    /// perform a core dump.
    ///
    /// Corresponds to POSIX signal `SIGQUIT`.
    Quit,
}

impl Signal {
    /// Convert a [`mio_signals::Signal`] into our own `Signal`.
    pub(super) fn from_mio(signal: mio_signals::Signal) -> Signal {
        match signal {
            mio_signals::Signal::Interrupt => Signal::Interrupt,
            mio_signals::Signal::Terminate => Signal::Terminate,
            mio_signals::Signal::Quit => Signal::Quit,
        }
    }

    /// Whether or not the `Signal` is considered a "stopping" signal.
    pub(super) fn should_stop(self) -> bool {
        true
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let alternate = f.alternate();
        f.write_str(match (self, alternate) {
            (Signal::Interrupt, false) => "interrupt",
            (Signal::Interrupt, true) => "interrupt (SIGINT)",
            (Signal::Terminate, false) => "terminate",
            (Signal::Terminate, true) => "terminate (SIGTERM)",
            (Signal::Quit, false) => "quit",
            (Signal::Quit, true) => "quit (SIGQUIT)",
        })
    }
}
