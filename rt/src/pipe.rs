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
//! use heph_rt::{self as rt, pipe};
//!
//! const DATA: &[u8] = b"Hello, world!";
//!
//! async fn process_handler<RT>(ctx: actor::Context<!, RT>) -> io::Result<()>
//!     where RT: rt::Access,
//! {
//!     let (sender, receiver) = pipe::new(ctx.runtime_ref())?;
//!
//!     // Write some data.
//!     sender.write_all(DATA).await?;
//!     drop(sender); // Close the sending side.
//!
//!     // And read the data back.
//!     let buf = receiver.read_n(Vec::with_capacity(DATA.len() + 1), DATA.len()).await?;
//!     assert_eq!(buf, DATA);
//!     Ok(())
//! }
//! #
//! # let actor_ref = heph_rt::test::try_spawn(
//! #     heph_rt::test::PanicSupervisor,
//! #     process_handler as fn(_) -> _,
//! #     (),
//! #     heph_rt::spawn::ActorOptions::default(),
//! # ).unwrap();
//! # heph_rt::test::join(&actor_ref, std::time::Duration::from_secs(1)).unwrap();
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
//! use heph_rt::{self as rt, pipe};
//!
//! const DATA: &[u8] = b"Hello, world!";
//!
//! async fn process_handler<RT>(ctx: actor::Context<!, RT>) -> io::Result<()>
//!     where RT: rt::Access,
//! {
//!     // Spawn the "echo" command that echos everything it reads from standard
//!     // in to standard out.
//!     let mut process = Command::new("echo")
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
//!     stdin.write_all(DATA).await?;
//!     drop(stdin); // Close standard in for the child process.
//! #   process.wait()?; // Needed to pass the test on macOS.
//!
//!     // And read the data back.
//!     let buf = stdout.read_n(Vec::with_capacity(DATA.len() + 1), DATA.len()).await?;
//!     assert_eq!(buf, DATA);
//!     Ok(())
//! }
//! #
//! # let actor_ref = heph_rt::test::try_spawn(
//! #     heph_rt::test::PanicSupervisor,
//! #     process_handler as fn(_) -> _,
//! #     (),
//! #     heph_rt::spawn::ActorOptions::default(),
//! # ).unwrap();
//! # heph_rt::test::join(&actor_ref, std::time::Duration::from_secs(1)).unwrap();
//! ```

use std::io;
use std::os::fd::{IntoRawFd, RawFd};
use std::process::{ChildStderr, ChildStdin, ChildStdout};

use a10::{AsyncFd, Extract};

use crate::access::Access;
use crate::io::{
    Buf, BufMut, BufMutSlice, BufSlice, BufWrapper, Read, ReadN, ReadNVectored, ReadVectored,
    Write, WriteAll, WriteAllVectored, WriteVectored,
};

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
    let r = unsafe { AsyncFd::new(fds[0], sq.clone()) };
    let w = unsafe { AsyncFd::new(fds[1], sq) };
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
        // Safety: `ChildStdin` is guaranteed to be a valid file descriptor.
        let fd = unsafe { AsyncFd::new(stdin.into_raw_fd(), rt.submission_queue()) };
        Ok(Sender { fd })
    }

    /// Write the bytes in `buf` into the pipe.
    ///
    /// Return the number of bytes written. This may we fewer than the length of
    /// `buf`. To ensure that all bytes are written use [`Sender::write_all`].
    pub async fn write<B: Buf>(&self, buf: B) -> io::Result<(B, usize)> {
        Write(self.fd.write(BufWrapper(buf)).extract()).await
    }

    /// Write the all bytes in `buf` into the pipe.
    ///
    /// If this fails to write all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    pub async fn write_all<B: Buf>(&self, buf: B) -> io::Result<B> {
        WriteAll(self.fd.write_all(BufWrapper(buf)).extract()).await
    }

    /// Write the bytes in `bufs` into the pipe.
    ///
    /// Return the number of bytes written. This may we fewer than the length of
    /// `bufs`. To ensure that all bytes are written use
    /// [`Sender::write_vectored_all`].
    pub async fn write_vectored<B: BufSlice<N>, const N: usize>(
        &self,
        bufs: B,
    ) -> io::Result<(B, usize)> {
        WriteVectored(self.fd.write_vectored(BufWrapper(bufs)).extract()).await
    }

    /// Write the all bytes in `bufs` into the pipe.
    ///
    /// If this fails to write all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    pub async fn write_vectored_all<B: BufSlice<N>, const N: usize>(
        &self,
        bufs: B,
    ) -> io::Result<B> {
        WriteAllVectored(self.fd.write_all_vectored(BufWrapper(bufs)).extract()).await
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
        // Safety: `ChildStdout` is guaranteed to be a valid file descriptor.
        let fd = unsafe { AsyncFd::new(stdout.into_raw_fd(), rt.submission_queue()) };
        Ok(Receiver { fd })
    }

    /// Convert a [`ChildStderr`] to a `Receiver`.
    pub fn from_child_stderr<RT>(rt: &RT, stderr: ChildStderr) -> io::Result<Receiver>
    where
        RT: Access,
    {
        // Safety: `ChildStderr` is guaranteed to be a valid file descriptor.
        let fd = unsafe { AsyncFd::new(stderr.into_raw_fd(), rt.submission_queue()) };
        Ok(Receiver { fd })
    }

    /// Read bytes from the pipe, writing them into `buf`.
    pub async fn read<B: BufMut>(&self, buf: B) -> io::Result<B> {
        Read(self.fd.read(BufWrapper(buf))).await
    }

    /// Read at least `n` bytes from the pipe, writing them into `buf`.
    ///
    /// This returns [`io::ErrorKind::UnexpectedEof`] if less than `n` bytes
    /// could be read.
    pub async fn read_n<B: BufMut>(&self, buf: B, n: usize) -> io::Result<B> {
        debug_assert!(
            buf.spare_capacity() >= n,
            "called `Receiver::read_n` with a buffer smaller than `n`",
        );
        ReadN(self.fd.read_n(BufWrapper(buf), n)).await
    }

    /// Read bytes from the pipe, writing them into `bufs`.
    pub async fn read_vectored<B: BufMutSlice<N>, const N: usize>(&self, bufs: B) -> io::Result<B> {
        ReadVectored(self.fd.read_vectored(BufWrapper(bufs))).await
    }

    /// Read at least `n` bytes from the pipe, writing them into `bufs`.
    pub async fn read_n_vectored<B: BufMutSlice<N>, const N: usize>(
        &self,
        bufs: B,
        n: usize,
    ) -> io::Result<B> {
        debug_assert!(
            bufs.total_spare_capacity() >= n,
            "called `Receiver::read_n_vectored` with buffers smaller than `n`"
        );
        ReadNVectored(self.fd.read_n_vectored(BufWrapper(bufs), n)).await
    }
}
