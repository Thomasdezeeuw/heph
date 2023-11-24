use std::convert::TryFrom;
use std::fmt;

use heph::messages::Terminate;

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
/// receive a process signal or they may not. This is due to OS limitations and
/// differences. Any manually spawned threads spawned after calling build should
/// not get a process signal.
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
    /// Quit signal.
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
    /// Child signal.
    ///
    /// This signal is sent to a process when a child process terminates, is
    /// stopped, or resumes after being stopped.
    ///
    /// Corresponds to POSIX signal `SIGCHLD`.
    Child,
    /// Alarm signal.
    ///
    /// This signal signals is sent to a process when a time limit is reached.
    /// This alarm is based on real or clock time.
    ///
    /// Also see [`Signal::VirtualAlarm`] and  [`Signal::Profile`].
    ///
    /// Corresponds to POSIX signal `SIGALRM`.
    Alarm,
    /// Virtual timer alarm signal.
    ///
    /// This signal signals is sent to a process when a time limit is reached.
    /// This alarm is based on CPU time.
    ///
    /// Also see [`Signal::Alarm`] and [`Signal::Profile`].
    ///
    /// Corresponds to POSIX signal `SIGVTALRM`.
    VirtualAlarm,
    /// Profiling timer signal.
    ///
    /// This signal signals is sent to a process when a time limit is reached.
    /// This alarm is based on CPU time used by the process and the system
    /// itself.
    ///
    /// Also see [`Signal::Alarm`] and [`Signal::VirtualAlarm`].
    ///
    /// Corresponds to POSIX signal `SIGPROF`.
    Profile,
    /// Continue signal.
    ///
    /// This signal sent to a process to continue (restart) after being
    /// previously paused by the `SIGSTOP` (stop) or `SIGTSTP` signal.
    ///
    /// Corresponds to POSIX signal `SIGCONT`.
    Continue,
    /// Hangup signal.
    ///
    /// This signal is sent to a process when its controlling terminal is
    /// closed. In modern systems, this signal usually means that the
    /// controlling pseudo or virtual terminal has been closed.
    ///
    /// Corresponds to POSIX signal `SIGHUP`.
    Hangup,
    /// Window change signal.
    ///
    /// This signal is sent to a process when its controlling terminal changes
    /// its size.
    ///
    /// Corresponds to POSIX signal `SIGWINCH`.
    WindowChange,
    /// Exceeded CPU signal.
    ///
    /// This signal is sent to a process when it has used up the CPU for a
    /// duration that exceeds a certain predetermined user-settable value.
    ///
    /// Corresponds to POSIX signal `SIGXCPU`.
    ExceededCpu,
    /// Excess file size signal.
    ///
    /// This signal is sent to a process when it grows a file that exceeds the
    /// maximum allowed size.
    ///
    /// Corresponds to POSIX signal `SIGXFSZ`.
    ExcessFileSize,
    /// Pipe signal.
    ///
    /// This signal is sent to a process when it attempts to write to a pipe
    /// without a process connected to the other end.
    ///
    /// Corresponds to POSIX signal `SIGPIPE`.
    Pipe,
    /// Urgent signal.
    ///
    /// This signal is sent to a process when a socket has urgent or out-of-band
    /// data available to read.
    ///
    /// Corresponds to POSIX signal `SIGURG`.
    Urgent,
    /// Bad system call signal.
    ///
    /// This signal is sent to a process when it passes a bad argument to a
    /// system call. This can also be received by applications violating the
    /// Linux Seccomp security rules configured to restrict them.
    ///
    /// Corresponds to POSIX signal `SIGSYS`.
    BadSystemCall,
    /// Trap signal.
    ///
    /// This signal is sent to a process when an exception (or trap) occurs: a
    /// condition that a debugger has requested to be informed of.
    ///
    /// Corresponds to POSIX signal `SIGTRAP`.
    Trap,
    /// Abort signal.
    ///
    /// This signal is sent to a process to tell it to abort, i.e. to terminate.
    /// This signal can also indicate that the CPU has executed an explicit
    /// "trap" instruction (without a defined function), or an unimplemented
    /// instruction (when emulation is unavailable).
    ///
    /// Corresponds to POSIX signal `SIGABRT`.
    Abort,
    /// Illegal signal.
    ///
    /// This signal is sent to a process when it attempts to execute an illegal,
    /// malformed, unknown, or privileged instruction.
    ///
    /// Corresponds to POSIX signal `SIGILL`.
    Illegal,
    /// Segmentation violation signal.
    ///
    /// This signal is sent to a process when it makes an invalid virtual memory
    /// reference, or segmentation fault.
    ///
    /// Corresponds to POSIX signal `SIGSEGV`.
    SegmentationViolation,
    /// Bus signal.
    ///
    /// This signal is sent to a process when it causes a bus error. The
    /// conditions that lead to the signal being sent are, for example,
    /// incorrect memory access alignment or non-existent physical address.
    ///
    /// Corresponds to POSIX signal `SIGBUS`.
    Bus,
    /// Floating point error signal.
    ///
    /// This signal is sent to a process when an exceptional (but not
    /// necessarily erroneous) condition has been detected in the floating point
    /// or integer arithmetic hardware. This may include division by zero,
    /// floating point underflow or overflow, integer overflow, an invalid
    /// operation or an inexact computation. Behaviour may differ depending on
    /// hardware.
    ///
    /// Corresponds to POSIX signal `SIGFPE`.
    FloatingPointError,
    /// Terminal input for background process signal.
    ///
    /// This signal is sent to a process when it attempts to read from the tty
    /// while in the background.
    ///
    /// Corresponds to POSIX signal `SIGTTIN`.
    TerminalInputBackground,
    /// Terminal output for background process signal.
    ///
    /// This signal is sent to a process when it attempts to write to the tty
    /// while in the background.
    ///
    /// Corresponds to POSIX signal `SIGTTOU`.
    TerminalOutputBackground,
}

impl Signal {
    /// All signals supported by [`Signal`].
    pub(crate) const ALL: [Signal; 25] = [
        Signal::Interrupt,
        Signal::Terminate,
        Signal::Quit,
        Signal::User1,
        Signal::User2,
        Signal::Child,
        Signal::Alarm,
        Signal::VirtualAlarm,
        Signal::Profile,
        Signal::Continue,
        Signal::Hangup,
        Signal::WindowChange,
        Signal::ExceededCpu,
        Signal::ExcessFileSize,
        Signal::Pipe,
        Signal::Urgent,
        Signal::BadSystemCall,
        Signal::Trap,
        Signal::Abort,
        Signal::Illegal,
        Signal::SegmentationViolation,
        Signal::Bus,
        Signal::FloatingPointError,
        Signal::TerminalInputBackground,
        Signal::TerminalOutputBackground,
    ];

    /// Turns a signal number into a `Signal`. Returns `None` if we don't have a
    /// variant for the signal number.
    pub(crate) const fn from_signo(signo: libc::c_int) -> Option<Signal> {
        Some(match signo {
            libc::SIGINT => Signal::Interrupt,
            libc::SIGTERM => Signal::Terminate,
            libc::SIGQUIT => Signal::Quit,
            libc::SIGUSR1 => Signal::User1,
            libc::SIGUSR2 => Signal::User2,
            libc::SIGCHLD => Signal::Child,
            libc::SIGALRM => Signal::Alarm,
            libc::SIGVTALRM => Signal::VirtualAlarm,
            libc::SIGPROF => Signal::Profile,
            libc::SIGCONT => Signal::Continue,
            libc::SIGHUP => Signal::Hangup,
            libc::SIGWINCH => Signal::WindowChange,
            libc::SIGXCPU => Signal::ExceededCpu,
            libc::SIGXFSZ => Signal::ExcessFileSize,
            libc::SIGPIPE => Signal::Pipe,
            libc::SIGURG => Signal::Urgent,
            libc::SIGSYS => Signal::BadSystemCall,
            libc::SIGTRAP => Signal::Trap,
            libc::SIGABRT => Signal::Abort,
            libc::SIGILL => Signal::Illegal,
            libc::SIGSEGV => Signal::SegmentationViolation,
            libc::SIGBUS => Signal::Bus,
            libc::SIGFPE => Signal::FloatingPointError,
            libc::SIGTTIN => Signal::TerminalInputBackground,
            libc::SIGTTOU => Signal::TerminalOutputBackground,
            _ => return None,
        })
    }

    /// Returns the signal as signal number.
    pub(crate) const fn to_signo(self) -> libc::c_int {
        match self {
            Signal::Interrupt => libc::SIGINT,
            Signal::Terminate => libc::SIGTERM,
            Signal::Quit => libc::SIGQUIT,
            Signal::User1 => libc::SIGUSR1,
            Signal::User2 => libc::SIGUSR2,
            Signal::Child => libc::SIGCHLD,
            Signal::Alarm => libc::SIGALRM,
            Signal::VirtualAlarm => libc::SIGVTALRM,
            Signal::Profile => libc::SIGPROF,
            Signal::Continue => libc::SIGCONT,
            Signal::Hangup => libc::SIGHUP,
            Signal::WindowChange => libc::SIGWINCH,
            Signal::ExceededCpu => libc::SIGXCPU,
            Signal::ExcessFileSize => libc::SIGXFSZ,
            Signal::Pipe => libc::SIGPIPE,
            Signal::Urgent => libc::SIGURG,
            Signal::BadSystemCall => libc::SIGSYS,
            Signal::Trap => libc::SIGTRAP,
            Signal::Abort => libc::SIGABRT,
            Signal::Illegal => libc::SIGILL,
            Signal::SegmentationViolation => libc::SIGSEGV,
            Signal::Bus => libc::SIGBUS,
            Signal::FloatingPointError => libc::SIGFPE,
            Signal::TerminalInputBackground => libc::SIGTTIN,
            Signal::TerminalOutputBackground => libc::SIGTTOU,
        }
    }

    /// Whether or not the `Signal` is considered a "stopping" signal.
    pub const fn should_stop(self) -> bool {
        matches!(self, Signal::Interrupt | Signal::Terminate | Signal::Quit)
    }

    /// Returns a human readable name for the signal.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Signal::Interrupt => "interrupt",
            Signal::Terminate => "terminate",
            Signal::Quit => "quit",
            Signal::User1 => "user-1",
            Signal::User2 => "user-2",
            Signal::Child => "child",
            Signal::Alarm => "alarm",
            Signal::VirtualAlarm => "virtual-alarm",
            Signal::Profile => "profile",
            Signal::Continue => "continue",
            Signal::Hangup => "handup",
            Signal::WindowChange => "window-change",
            Signal::ExceededCpu => "exceeded-cpu",
            Signal::ExcessFileSize => "excess-file-size",
            Signal::Pipe => "pipe",
            Signal::Urgent => "urgent",
            Signal::BadSystemCall => "bad-syscall",
            Signal::Trap => "trap",
            Signal::Abort => "abort",
            Signal::Illegal => "illegal",
            Signal::SegmentationViolation => "segfault",
            Signal::Bus => "bus",
            Signal::FloatingPointError => "floating-point-error",
            Signal::TerminalInputBackground => "terminal-input",
            Signal::TerminalOutputBackground => "terminal-output",
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
            Signal::Child => "SIGCHLD",
            Signal::Alarm => "SIGALRM",
            Signal::VirtualAlarm => "SIGVTALRM",
            Signal::Profile => "SIGPROF",
            Signal::Continue => "SIGCONT",
            Signal::Hangup => "SIGHUP",
            Signal::WindowChange => "SIGWINCH",
            Signal::ExceededCpu => "SIGXCPU",
            Signal::ExcessFileSize => "SIGXFSZ",
            Signal::Pipe => "SIGPIPE",
            Signal::Urgent => "SIGURG",
            Signal::BadSystemCall => "SIGSYS",
            Signal::Trap => "SIGTRAP",
            Signal::Abort => "SIGABRT",
            Signal::Illegal => "SIGILL",
            Signal::SegmentationViolation => "SIGSEGV",
            Signal::Bus => "SIGBUS",
            Signal::FloatingPointError => "SIGFPE",
            Signal::TerminalInputBackground => "SIGTTIN",
            Signal::TerminalOutputBackground => "SIGTTOU",
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

impl TryFrom<Signal> for Terminate {
    type Error = ();

    /// Converts the `signal` into a `Terminate` message if
    /// [`Signal::should_stop`] returns true, fails for all other signals (by
    /// returning `Err(())`).
    fn try_from(signal: Signal) -> Result<Self, Self::Error> {
        if signal.should_stop() {
            Ok(Terminate)
        } else {
            Err(())
        }
    }
}
