//! Unix pipes.
//!
//! To create a new pipe use the [`pipe`] function. It will return two
//! [`AsyncFd`]s, the sending and receiving side.
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
//! use heph_rt::pipe::pipe;
//! use heph_rt::{self as rt};
//!
//! const DATA: &[u8] = b"Hello, world!";
//!
//! async fn actor<RT>(ctx: actor::Context<!, RT>) -> io::Result<()>
//!     where RT: rt::Access,
//! {
//!     let [receiver, sender] = pipe(ctx.runtime_ref())?;
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
//! # #[cfg(any(test, feature = "test"))]
//! # heph_rt::test::block_on_local_actor(heph::actor::actor_fn(actor), ());
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
//! use heph_rt::{self as rt};
//! use heph_rt::fd::AsyncFd;
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
//!     let stdin: AsyncFd = AsyncFd::new(process.stdin.take().unwrap().into(), ctx.runtime_ref().sq());
//!     let stdout: AsyncFd = AsyncFd::new(process.stdout.take().unwrap().into(), ctx.runtime_ref().sq());
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
//! # #[cfg(any(test, feature = "test"))]
//! # heph_rt::test::block_on_local_actor(heph::actor::actor_fn(process_handler), ());
//! ```

use std::io;
use std::os::fd::RawFd;

use a10::AsyncFd;

use crate::access::Access;

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
pub fn pipe<RT>(rt: &RT) -> io::Result<[AsyncFd; 2]>
where
    RT: Access,
{
    let mut fds: [RawFd; 2] = [-1, -1];
    let _ = syscall!(pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC))?;

    let sq = rt.sq();
    // SAFETY: we just initialised the `fds` above.
    let r = unsafe { AsyncFd::from_raw_fd(fds[0], sq.clone()) };
    let w = unsafe { AsyncFd::from_raw_fd(fds[1], sq) };
    Ok([r, w])
}
