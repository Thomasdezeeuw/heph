/// Process signal.
///
/// Actors can receive signals by calling [`ActorSystemRef::receive_signals`]
/// with their actor reference.
///
/// Synchronous actors receive signals automatically if they implement the
/// required trait bounds, see [`SyncActor`] for more details.
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
    /// Convert to a byte, parsing using [`from_byte`].
    ///
    /// [`from_byte`]: Signal::from_byte
    pub(super) fn to_byte(self) -> u8 {
        use Signal::*;
        match self {
            Interrupt => 1,
            Terminate => 2,
            Quit => 3,
        }
    }

    /// Parse a signal converted to a byte using [`to_byte`].
    ///
    /// [`to_byte`]: Signal::to_byte
    pub(super) fn from_byte(byte: u8) -> Option<Signal> {
        use Signal::*;
        match byte {
            1 => Some(Interrupt),
            2 => Some(Terminate),
            3 => Some(Quit),
            _ => None,
        }
    }

    /// Convert a [`mio_signals::Signal`] into our own `Signal`.
    pub(super) fn from_mio(signal: mio_signals::Signal) -> Signal {
        match signal {
            mio_signals::Signal::Interrupt => Signal::Interrupt,
            mio_signals::Signal::Terminate => Signal::Terminate,
            mio_signals::Signal::Quit => Signal::Quit,
        }
    }
}
