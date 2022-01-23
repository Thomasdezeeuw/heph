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
//! # Notes
//!
//! Both the [`Sender`] and [`Receiver`] types are [bound] to an actor. See the
//! [`actor::Bound`] trait for more information.
//!
//! [bound]: crate::actor::Bound
//! [`actor::Bound`]: crate::actor::Bound
//!
//! # Examples
//!
//! Creating a new Unix pipe.
//!
//! ```
//! # #![feature(never_type)]
//! use std::io;
//!
//! use heph::{unix, actor, rt};
//!
//! const DATA: &[u8] = b"Hello, world!";
//!
//! async fn process_handler<RT>(mut ctx: actor::Context<!, RT>) -> io::Result<()>
//!     where RT: rt::Access,
//! {
//!     let (mut sender, mut receiver) = unix::pipe::new(&mut ctx)?;
//!
//!     // Write some data.
//!     sender.write_all(DATA).await?;
//!     drop(sender); // Close the sending side.
//!
//!     // And read the data back.
//!     let mut buf = Vec::with_capacity(DATA.len() + 1);
//!     receiver.read_n(&mut buf, DATA.len()).await?;
//!     assert_eq!(buf, DATA);
//!     Ok(())
//! }
//! #
//! # let actor_ref = heph::test::try_spawn(
//! #     heph::test::PanicSupervisor,
//! #     process_handler as fn(_) -> _,
//! #     (),
//! #     heph::spawn::ActorOptions::default(),
//! # ).unwrap();
//! # heph::test::join(&actor_ref, std::time::Duration::from_secs(1)).unwrap();
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
//! use heph::{unix, actor, rt};
//!
//! const DATA: &[u8] = b"Hello, world!";
//!
//! async fn process_handler<RT>(mut ctx: actor::Context<!, RT>) -> io::Result<()>
//!     where RT: rt::Access,
//! {
//!     // Spawn a "echo" that echo everything read from standard in to standard
//!     // out.
//!     let mut process = Command::new("echo")
//!         .stdin(Stdio::piped())
//!         .stdout(Stdio::piped())
//!         .stderr(Stdio::null())
//!         .spawn()?;
//!
//!     // Create our process standard in and out.
//!     let mut stdin = unix::pipe::Sender::from_child_stdin(&mut ctx, process.stdin.take().unwrap())?;
//!     let mut stdout = unix::pipe::Receiver::from_child_stdout(&mut ctx, process.stdout.take().unwrap())?;
//!
//!     // Write some data.
//!     stdin.write_all(DATA).await?;
//!     drop(stdin); // Close standard in for the child process.
//! #   process.wait()?; // Needed to pass the test on macOS.
//!
//!     // And read the data back.
//!     let mut buf = Vec::with_capacity(DATA.len() + 1);
//!     stdout.read_n(&mut buf, DATA.len()).await?;
//!     assert_eq!(buf, DATA);
//!     Ok(())
//! }
//! #
//! # let actor_ref = heph::test::try_spawn(
//! #     heph::test::PanicSupervisor,
//! #     process_handler as fn(_) -> _,
//! #     (),
//! #     heph::spawn::ActorOptions::default(),
//! # ).unwrap();
//! # heph::test::join(&actor_ref, std::time::Duration::from_secs(1)).unwrap();
//! ```

use std::future::Future;
use std::io::{self, IoSlice};
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::process::{ChildStderr, ChildStdin, ChildStdout};
use std::task::{self, Poll};

use mio::unix::pipe;
use mio::Interest;

use crate::bytes::{Bytes, BytesVectored, MaybeUninitSlice};
use crate::{actor, rt};

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
pub fn new<M, RT>(ctx: &mut actor::Context<M, RT>) -> io::Result<(Sender, Receiver)>
where
    RT: rt::Access,
{
    let (mut sender, mut receiver) = pipe::new()?;

    let rt = ctx.runtime();
    rt.register(&mut sender, Interest::WRITABLE)?;
    rt.register(&mut receiver, Interest::READABLE)?;

    Ok((Sender { inner: sender }, Receiver { inner: receiver }))
}

/// Sending end of an Unix pipe.
///
/// Created by calling [`new`] or converted from [`ChildStdin`].
#[derive(Debug)]
pub struct Sender {
    inner: pipe::Sender,
}

impl Sender {
    /// Convert a [`ChildStdin`] to a `Sender`.
    pub fn from_child_stdin<M, RT>(
        ctx: &mut actor::Context<M, RT>,
        stdin: ChildStdin,
    ) -> io::Result<Sender>
    where
        RT: rt::Access,
    {
        let mut sender = pipe::Sender::from(stdin);
        sender.set_nonblocking(true)?;
        ctx.runtime().register(&mut sender, Interest::WRITABLE)?;
        Ok(Sender { inner: sender })
    }

    /// Attempt to write the bytes in `buf` into the pipe.
    ///
    /// If no bytes can currently be written this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`Sender::write`] or [`Sender::write_all`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::Write::write(&mut self.inner, buf)
    }

    /// Write the bytes in `buf` into the pipe.
    ///
    /// Return the number of bytes written. This may we fewer then the length of
    /// `buf`. To ensure that all bytes are written use [`Sender::write_all`].
    pub fn write<'a, 'b>(&'a mut self, buf: &'b [u8]) -> Write<'a, 'b> {
        Write { sender: self, buf }
    }

    /// Write the all bytes in `buf` into the pipe.
    ///
    /// If this fails to write all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    pub fn write_all<'a, 'b>(&'a mut self, buf: &'b [u8]) -> WriteAll<'a, 'b> {
        WriteAll { sender: self, buf }
    }

    /// Attempt to write the bytes in `bufs` into the pipe.
    ///
    /// If no bytes can currently be written this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`Sender::write_vectored`] or [`Sender::write_vectored_all`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        io::Write::write_vectored(&mut self.inner, bufs)
    }

    /// Write the bytes in `bufs` intoto the pipe.
    ///
    /// Return the number of bytes written. This may we fewer then the length of
    /// `bufs`. To ensure that all bytes are written use
    /// [`Sender::write_vectored_all`].
    pub fn write_vectored<'a, 'b>(
        &'a mut self,
        bufs: &'b mut [IoSlice<'b>],
    ) -> WriteVectored<'a, 'b> {
        WriteVectored { sender: self, bufs }
    }

    /// Write the all bytes in `bufs` into the pipe.
    ///
    /// If this fails to write all bytes (this happens if a write returns
    /// `Ok(0)`) this will return [`io::ErrorKind::WriteZero`].
    pub fn write_vectored_all<'a, 'b>(
        &'a mut self,
        bufs: &'b mut [IoSlice<'b>],
    ) -> WriteVectoredAll<'a, 'b> {
        WriteVectoredAll { sender: self, bufs }
    }
}

/// The [`Future`] behind [`Sender::write`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Write<'a, 'b> {
    sender: &'a mut Sender,
    buf: &'b [u8],
}

impl<'a, 'b> Future for Write<'a, 'b> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Write { sender, buf } = Pin::into_inner(self);
        try_io!(sender.try_write(*buf))
    }
}

/// The [`Future`] behind [`Sender::write_all`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct WriteAll<'a, 'b> {
    sender: &'a mut Sender,
    buf: &'b [u8],
}

impl<'a, 'b> Future for WriteAll<'a, 'b> {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let WriteAll { sender, buf } = Pin::into_inner(self);
        loop {
            match sender.try_write(*buf) {
                Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Ok(n) if buf.len() <= n => return Poll::Ready(Ok(())),
                Ok(n) => {
                    *buf = &buf[n..];
                    // Try to write some more bytes.
                    continue;
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }
}

/// The [`Future`] behind [`Sender::write_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct WriteVectored<'a, 'b> {
    sender: &'a mut Sender,
    bufs: &'b mut [IoSlice<'b>],
}

impl<'a, 'b> Future for WriteVectored<'a, 'b> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let WriteVectored { sender, bufs } = Pin::into_inner(self);
        try_io!(sender.try_write_vectored(*bufs))
    }
}

/// The [`Future`] behind [`Sender::write_vectored_all`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct WriteVectoredAll<'a, 'b> {
    sender: &'a mut Sender,
    bufs: &'b mut [IoSlice<'b>],
}

impl<'a, 'b> Future for WriteVectoredAll<'a, 'b> {
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let WriteVectoredAll { sender, bufs } = Pin::into_inner(self);
        while !bufs.is_empty() {
            match sender.try_write_vectored(*bufs) {
                Ok(0) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Ok(n) => IoSlice::advance_slices(bufs, n),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => return Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Poll::Ready(Err(err)),
            }
        }
        Poll::Ready(Ok(()))
    }
}

impl<RT: rt::Access> actor::Bound<RT> for Sender {
    type Error = io::Error;

    fn bind_to<M>(&mut self, ctx: &mut actor::Context<M, RT>) -> io::Result<()> {
        ctx.runtime()
            .reregister(&mut self.inner, Interest::WRITABLE)
    }
}

/// Receiving end of an Unix pipe.
///
/// Created by calling [`new`] or converted from [`ChildStdout`] or
/// [`ChildStderr`].
#[derive(Debug)]
pub struct Receiver {
    inner: pipe::Receiver,
}

impl Receiver {
    /// Convert a [`ChildStdout`] to a `Receiver`.
    pub fn from_child_stdout<M, RT>(
        ctx: &mut actor::Context<M, RT>,
        stdout: ChildStdout,
    ) -> io::Result<Receiver>
    where
        RT: rt::Access,
    {
        let mut receiver = pipe::Receiver::from(stdout);
        receiver.set_nonblocking(true)?;
        ctx.runtime().register(&mut receiver, Interest::READABLE)?;
        Ok(Receiver { inner: receiver })
    }

    /// Convert a [`ChildStderr`] to a `Receiver`.
    pub fn from_child_stderr<M, RT>(
        ctx: &mut actor::Context<M, RT>,
        stderr: ChildStderr,
    ) -> io::Result<Receiver>
    where
        RT: rt::Access,
    {
        let mut receiver = pipe::Receiver::from(stderr);
        receiver.set_nonblocking(true)?;
        ctx.runtime().register(&mut receiver, Interest::READABLE)?;
        Ok(Receiver { inner: receiver })
    }

    /// Attempt to read bytes from the pipe, writing them into `buf`.
    ///
    /// If no bytes can currently be read this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`Receiver::read`] or [`Receiver::read_n`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_read<B>(&mut self, mut buf: B) -> io::Result<usize>
    where
        B: Bytes,
    {
        debug_assert!(
            buf.has_spare_capacity(),
            "called `Receiver::try_read` with an empty buffer"
        );
        // SAFETY: This is unsound.
        // However Mio passes the buffer directly to the OS without reading any
        // bytes, so it shouldn't invoke any UB.
        let buf_bytes = unsafe { &mut *(buf.as_bytes() as *mut [MaybeUninit<u8>] as *mut [u8]) };
        io::Read::read(&mut self.inner, buf_bytes).map(|read| {
            // Safety: just read the bytes.
            unsafe { buf.update_length(read) }
            read
        })
    }

    /// Read bytes from the pipe, writing them into `buf`.
    pub fn read<'a, B>(&'a mut self, buf: B) -> Read<'a, B>
    where
        B: Bytes,
    {
        Read {
            receiver: self,
            buf,
        }
    }

    /// Read at least `n` bytes from the pipe, writing them into `buf`.
    ///
    /// This returns a [`Future`] that receives at least `n` bytes from the
    /// `Receiver` and writes them into buffer `B`, or returns
    /// [`io::ErrorKind::UnexpectedEof`] if less then `n` bytes could be read.
    pub fn read_n<'a, B>(&'a mut self, buf: B, n: usize) -> ReadN<'a, B>
    where
        B: Bytes,
    {
        debug_assert!(
            buf.spare_capacity() >= n,
            "called `Reader::read_n` with a buffer smaller then `n`",
        );
        ReadN {
            receiver: self,
            buf,
            left: n,
        }
    }

    /// Attempt to read bytes from the pipe, writing them into `bufs`.
    ///
    /// If no bytes can currently be read this will return an error with the
    /// [kind] set to [`ErrorKind::WouldBlock`]. Most users should prefer to use
    /// [`Receiver::read_vectored`] or [`Receiver::read_n_vectored`].
    ///
    /// [kind]: io::Error::kind
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_read_vectored<B>(&mut self, mut bufs: B) -> io::Result<usize>
    where
        B: BytesVectored,
    {
        debug_assert!(
            bufs.has_spare_capacity(),
            "called `Receiver::try_read_vectored` with empty buffers"
        );
        let mut buffers = bufs.as_bufs();
        let bufs_bytes = unsafe { MaybeUninitSlice::as_io(buffers.as_mut()) };
        match io::Read::read_vectored(&mut self.inner, bufs_bytes) {
            Ok(read) => {
                drop(buffers);
                // Safety: just read the bytes.
                unsafe { bufs.update_lengths(read) }
                Ok(read)
            }
            Err(err) => Err(err),
        }
    }

    /// Read bytes from the pipe, writing them into `bufs`.
    pub fn read_vectored<B>(&mut self, bufs: B) -> ReadVectored<'_, B>
    where
        B: BytesVectored,
    {
        debug_assert!(
            bufs.has_spare_capacity(),
            "called `Receiver::read_vectored` with empty buffers"
        );
        ReadVectored {
            receiver: self,
            bufs,
        }
    }

    /// Read at least `n` bytes from the pipe, writing them into `bufs`.
    pub fn read_n_vectored<B>(&mut self, bufs: B, n: usize) -> ReadNVectored<'_, B>
    where
        B: BytesVectored,
    {
        debug_assert!(
            bufs.spare_capacity() >= n,
            "called `Receiver::read_n_vectored` with a buffer smaller then `n`"
        );
        ReadNVectored {
            receiver: self,
            bufs,
            left: n,
        }
    }
}

/// The [`Future`] behind [`Receiver::read`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Read<'b, B> {
    receiver: &'b mut Receiver,
    buf: B,
}

impl<'b, B> Future for Read<'b, B>
where
    B: Bytes + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let Read { receiver, buf } = Pin::into_inner(self);
        try_io!(receiver.try_read(&mut *buf))
    }
}

/// The [`Future`] behind [`Receiver::read_n`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct ReadN<'b, B> {
    receiver: &'b mut Receiver,
    buf: B,
    left: usize,
}

impl<'b, B> Future for ReadN<'b, B>
where
    B: Bytes + Unpin,
{
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let ReadN {
            receiver,
            buf,
            left,
        } = Pin::into_inner(self);
        loop {
            match receiver.try_read(&mut *buf) {
                Ok(0) => return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into())),
                Ok(n) if *left <= n => return Poll::Ready(Ok(())),
                Ok(n) => {
                    *left -= n;
                    // Try to read some more bytes.
                    continue;
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }
}

/// The [`Future`] behind [`Receiver::read_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct ReadVectored<'b, B> {
    receiver: &'b mut Receiver,
    bufs: B,
}

impl<'b, B> Future for ReadVectored<'b, B>
where
    B: BytesVectored + Unpin,
{
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let ReadVectored { receiver, bufs } = Pin::into_inner(self);
        try_io!(receiver.try_read_vectored(&mut *bufs))
    }
}

/// The [`Future`] behind [`Receiver::read_n_vectored`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct ReadNVectored<'b, B> {
    receiver: &'b mut Receiver,
    bufs: B,
    left: usize,
}

impl<'b, B> Future for ReadNVectored<'b, B>
where
    B: BytesVectored + Unpin,
{
    type Output = io::Result<()>;

    fn poll(self: Pin<&mut Self>, _: &mut task::Context<'_>) -> Poll<Self::Output> {
        let ReadNVectored {
            receiver,
            bufs,
            left,
        } = Pin::into_inner(self);
        loop {
            match receiver.try_read_vectored(&mut *bufs) {
                Ok(0) => return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into())),
                Ok(n) if *left <= n => return Poll::Ready(Ok(())),
                Ok(n) => {
                    *left -= n;
                    // Try to read some more bytes.
                    continue;
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => break Poll::Pending,
                Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => break Poll::Ready(Err(err)),
            }
        }
    }
}

impl<RT: rt::Access> actor::Bound<RT> for Receiver {
    type Error = io::Error;

    fn bind_to<M>(&mut self, ctx: &mut actor::Context<M, RT>) -> io::Result<()> {
        ctx.runtime()
            .reregister(&mut self.inner, Interest::READABLE)
    }
}
