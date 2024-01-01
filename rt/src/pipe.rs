//! Unix pipes.
//!
//! To create a new pipe use the [`new`] function. `new` will return a
//! [`Sender`] and [`Receiver`], which are two sides of the pipe.
//!
//! In addition to creating a new pipe it's also possible to create a pipe from
//! a process' standard I/O when [spawning another process]. For this use
//! [`Sender::from_child_stdin`], [`Receiver::from_child_stdout`] and
//! [`Receiver::from_child_stderr`] methods. See the example below.
//!
//! [spawning another process]: std::process::Command
//!
//! # Examples
//!
//! Creating a new Unix pipe.
//!
//! ```
//! # #![feature(never_type)]
//! use std::io;
//!
//! use heph::actor;
//! use heph_rt::io::{Read, Write};
//! use heph_rt::{self as rt, pipe};
//!
//! const DATA: &[u8] = b"Hello, world!";
//!
//! async fn actor<RT>(ctx: actor::Context<!, RT>) -> io::Result<()>
//!     where RT: rt::Access,
//! {
//!     let (sender, receiver) = pipe::new(ctx.runtime_ref())?;
//!
//!     // Write some data.
//!     (&sender).write_all(DATA).await?;
//!     drop(sender); // Close the sending side.
//!
//!     // And read the data back.
//!     let buf = (&receiver).read_n(Vec::with_capacity(DATA.len() + 1), DATA.len()).await?;
//!     assert_eq!(buf, DATA);
//!     Ok(())
//! }
//! #
//! # heph_rt::test::block_on_local_actor(heph::actor::actor_fn(actor), ()).unwrap();
//! ```
//!
//! Spawn a process using a pipe for standard in, out and error of the spawned
//! process.
//!
//! ```
//! # #![feature(never_type)]
//! use std::io;
//! use std::process::{Command, Stdio};
//!
//! use heph::actor;
//! use heph_rt::io::{Read, Write};
//! use heph_rt::{self as rt, pipe};
//!
//! const DATA: &[u8] = b"Hello, world!";
//!
//! async fn process_handler<RT>(ctx: actor::Context<!, RT>) -> io::Result<()>
//!     where RT: rt::Access,
//! {
//!     // Spawn the "echo" command that echos everything it reads from standard
//!     // in to standard out.
//!     let mut process = Command::new("cat")
//!         .stdin(Stdio::piped())
//!         .stdout(Stdio::piped())
//!         .stderr(Stdio::null())
//!         .spawn()?;
//!
//!     // Create our process standard in and out.
//!     let stdin = pipe::Sender::from_child_stdin(ctx.runtime_ref(), process.stdin.take().unwrap())?;
//!     let stdout = pipe::Receiver::from_child_stdout(ctx.runtime_ref(), process.stdout.take().unwrap())?;
//!
//!     // Write some data.
//!     (&stdin).write_all(DATA).await?;
//!     drop(stdin); // Close standard in for the child process.
//! #   process.wait()?; // Needed to pass the test on macOS.
//!
//!     // And read the data back.
//!     let buf = (&stdout).read_n(Vec::with_capacity(DATA.len() + 1), DATA.len()).await?;
//!     assert_eq!(buf, DATA);
//!     Ok(())
//! }
//! #
//! # heph_rt::test::block_on_local_actor(heph::actor::actor_fn(process_handler), ()).unwrap();
//! ```

use std::io;
use std::os::fd::{AsFd, BorrowedFd, IntoRawFd, RawFd};
use std::process::{ChildStderr, ChildStdin, ChildStdout};

use a10::AsyncFd;

use crate::access::Access;
use crate::io::{impl_read, impl_write};

/// Create a new Unix pipe.
///
/// This is a wrapper around Unix's [`pipe(2)`] system call and can be used as
/// inter-process or thread communication channel.
///
/// This channel may be created before forking the process and then one end used
/// in each process, e.g. the parent process has the sending end to send
/// commands to the child process.
///
/// [`pipe(2)`]: https://pubs.opengroup.org/onlinepubs/9699919799/functions/pipe.html
pub fn new<RT>(rt: &RT) -> io::Result<(Sender, Receiver)>
where
    RT: Access,
{
    let mut fds: [RawFd; 2] = [-1, -1];
    let _ = syscall!(pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC))?;

    let sq = rt.submission_queue();
    // SAFETY: we just initialised the `fds` above.
    let r = unsafe { AsyncFd::from_raw_fd(fds[0], sq.clone()) };
    let w = unsafe { AsyncFd::from_raw_fd(fds[1], sq) };
    Ok((Sender { fd: w }, Receiver { fd: r }))
}

/// Sending end of an Unix pipe.
///
/// Created by calling [`new`] or converted from [`ChildStdin`].
#[derive(Debug)]
pub struct Sender {
    fd: AsyncFd,
}

impl Sender {
    /// Convert a [`ChildStdin`] to a `Sender`.
    pub fn from_child_stdin<RT>(rt: &RT, stdin: ChildStdin) -> io::Result<Sender>
    where
        RT: Access,
    {
        // SAFETY: `ChildStdin` is guaranteed to be a valid file descriptor.
        let fd = unsafe { AsyncFd::from_raw_fd(stdin.into_raw_fd(), rt.submission_queue()) };
        Ok(Sender { fd })
    }

    /// Creates a new independently owned `Sender` that shares the same
    /// underlying file descriptor as the existing `Sender`.
    pub fn try_clone(&self) -> io::Result<Sender> {
        Ok(Sender {
            fd: self.fd.try_clone()?,
        })
    }
}

impl_write!(Sender, &Sender);

impl AsFd for Sender {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// Receiving end of an Unix pipe.
///
/// Created by calling [`new`] or converted from [`ChildStdout`] or
/// [`ChildStderr`].
#[derive(Debug)]
pub struct Receiver {
    fd: AsyncFd,
}

impl Receiver {
    /// Convert a [`ChildStdout`] to a `Receiver`.
    pub fn from_child_stdout<RT>(rt: &RT, stdout: ChildStdout) -> io::Result<Receiver>
    where
        RT: Access,
    {
        // SAFETY: `ChildStdout` is guaranteed to be a valid file descriptor.
        let fd = unsafe { AsyncFd::from_raw_fd(stdout.into_raw_fd(), rt.submission_queue()) };
        Ok(Receiver { fd })
    }

    /// Convert a [`ChildStderr`] to a `Receiver`.
    pub fn from_child_stderr<RT>(rt: &RT, stderr: ChildStderr) -> io::Result<Receiver>
    where
        RT: Access,
    {
        // SAFETY: `ChildStderr` is guaranteed to be a valid file descriptor.
        let fd = unsafe { AsyncFd::from_raw_fd(stderr.into_raw_fd(), rt.submission_queue()) };
        Ok(Receiver { fd })
    }

    /// Creates a new independently owned `Receiver` that shares the same
    /// underlying file descriptor as the existing `Receiver`.
    pub fn try_clone(&self) -> io::Result<Receiver> {
        Ok(Receiver {
            fd: self.fd.try_clone()?,
        })
    }
}

impl_read!(Receiver, &Receiver);

impl AsFd for Receiver {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
