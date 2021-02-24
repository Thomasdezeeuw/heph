use std::fmt;

/// Process signal.
///
/// Actors can receive signals by calling [`Runtime::receive_signals`] or
/// [`RuntimeRef::receive_signals`] with their actor reference.
///
/// [`Runtime::receive_signals`]: crate::Runtime::receive_signals
/// [`RuntimeRef::receive_signals`]: crate::RuntimeRef::receive_signals
/// [`SyncActor`]: crate::actor::SyncActor
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
    pub(super) const fn from_mio(signal: mio_signals::Signal) -> Signal {
        match signal {
            mio_signals::Signal::Interrupt => Signal::Interrupt,
            mio_signals::Signal::Terminate => Signal::Terminate,
            mio_signals::Signal::Quit => Signal::Quit,
        }
    }

    /// Whether or not the `Signal` is considered a "stopping" signal.
    #[allow(clippy::unused_self)] // When more signals are added `self` is needed.
    pub(super) const fn should_stop(self) -> bool {
        true
    }

    /// Returns a human readable name for the signal.
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Signal::Interrupt => "interrupt",
            Signal::Terminate => "terminate",
            Signal::Quit => "quit",
        }
    }

    /// Returns the name of the Posix constant of the signal.
    const fn as_posix(self) -> &'static str {
        match self {
            Signal::Interrupt => "SIGINT",
            Signal::Terminate => "SIGTERM",
            Signal::Quit => "SIGQUIT",
        }
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())?;
        if f.alternate() {
            f.write_str(" ")?;
            f.write_str(self.as_posix())?;
        }
        Ok(())
    }
}
