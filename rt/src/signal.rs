use std::fmt;

/// Process signal.
///
/// All actors can receive process signals by calling
/// [`Runtime::receive_signals`] or [`RuntimeRef::receive_signals`] with their
/// actor reference. This causes all process signals to be relayed to the actor
/// which should handle them accordingly.
///
/// [`Runtime::receive_signals`]: crate::Runtime::receive_signals
/// [`RuntimeRef::receive_signals`]: crate::RuntimeRef::receive_signals
///
/// # Notes
///
/// What happens to threads spawned outside of Heph's control, i.e. manually
/// spawned, before calling [`rt::Setup::build`] is unspecified. They may still
/// receive a process signal or they may not. This is due to platform
/// limitations and differences. Any manually spawned threads spawned after
/// calling build should not get a process signal.
///
/// The runtime will only attempt to send the process signal to the actor once.
/// If the message can't be send it's **not** retried. Ensure that the inbox of
/// the actor has enough room to receive the message.
///
/// [`rt::Setup::build`]: crate::Setup::build
#[non_exhaustive]
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
    /// User-defined signal 1.
    ///
    /// Corresponds to POSIX signal `SIGUSR1`.
    User1,
    /// User-defined signal 2.
    ///
    /// Corresponds to POSIX signal `SIGUSR2`.
    ///
    /// # Notes
    ///
    /// The runtime will output various metrics about itself when it receives
    /// this signal.
    User2,
}

impl Signal {
    /// Convert a [`mio_signals::Signal`] into our own `Signal`.
    pub(super) const fn from_mio(signal: mio_signals::Signal) -> Signal {
        match signal {
            mio_signals::Signal::Interrupt => Signal::Interrupt,
            mio_signals::Signal::Terminate => Signal::Terminate,
            mio_signals::Signal::Quit => Signal::Quit,
            mio_signals::Signal::User1 => Signal::User1,
            mio_signals::Signal::User2 => Signal::User2,
        }
    }

    /// Whether or not the `Signal` is considered a "stopping" signal.
    pub(super) const fn should_stop(self) -> bool {
        match self {
            Signal::Interrupt | Signal::Terminate | Signal::Quit => true,
            Signal::User1 | Signal::User2 => false,
        }
    }

    /// Returns a human readable name for the signal.
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Signal::Interrupt => "interrupt",
            Signal::Terminate => "terminate",
            Signal::Quit => "quit",
            Signal::User1 => "user-1",
            Signal::User2 => "user-2",
        }
    }

    /// Returns the name of the Posix constant of the signal.
    const fn as_posix(self) -> &'static str {
        match self {
            Signal::Interrupt => "SIGINT",
            Signal::Terminate => "SIGTERM",
            Signal::Quit => "SIGQUIT",
            Signal::User1 => "SIGUSR1",
            Signal::User2 => "SIGUSR2",
        }
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())?;
        if f.alternate() {
            f.write_str(" (")?;
            f.write_str(self.as_posix())?;
            f.write_str(")")?;
        }
        Ok(())
    }
}
