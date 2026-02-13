//! These tests perform an end-to-end test based on the examples in the examples
//! directory.

#![feature(async_iterator, never_type, stmt_expr_attributes, write_all_vectored)]

use std::io::{self, Read};
use std::net::{SocketAddr, TcpStream};
use std::ops::{Deref, DerefMut};
use std::panic;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

#[path = "util/mod.rs"] // rustfmt can't find the file.
#[macro_use]
mod util;

use util::send_signal;

#[test]
fn test_1_hello_world() {
    let output = run_example_output("1_hello_world");
    assert_eq!(output, "Hello, World!\n");
}

#[test]
fn test_2_spawning_actors() {
    let output = run_example_output("2_spawning_actors");

    let expected = &[
        "Hello Alice from sync actor",
        "Hello Bob from thread-safe actor",
        "Hello Charlie from thread-local actor",
        "Hello Charlie from thread-local actor",
    ];
    unordered_output(expected, output);
}

#[test]
fn test_3_rpc() {
    let output = run_example_output("3_rpc");
    assert_eq!(
        output,
        "Got a RPC request: Ping\nGot a RPC response: Pong\n"
    );
}

#[test]
fn test_4_process_signals() {
    let mut child = run_example("4_process_signals");

    let want_greetings = &[
        "Got a message: Hello sync actor",
        "Got a message: Hello thread local actor",
        "Got a message: Hello thread safe actor",
    ];
    // All lines, including new line bytes + 1.
    let length = want_greetings.iter().map(|l| l.len()).sum::<usize>() + want_greetings.len();
    let mut output = vec![0; length + 1];

    // First read all greeting messages, ensuring the runtime has time to start
    // up.
    let mut n = 0;
    let mut max = want_greetings.len();
    #[rustfmt::skip]
    while n < length  {
        if max == 0 {
            panic!("too many reads, read only {n}/{length} bytes");
        }
        max -= 1;
        n += child.inner.stdout.as_mut().unwrap().read(&mut output[n..]).unwrap();
    }
    assert_eq!(n, length);
    output.truncate(n);
    let output = String::from_utf8(output).unwrap();

    unordered_output(want_greetings, output);

    // After we know the runtime started we can send it a process signal to
    // start the actual test.
    if let Err(err) = send_signal(child.inner.id(), libc::SIGINT) {
        panic!("unexpected error sending signal to process: {err}");
    }

    // Read the remainder of the output, expecting the shutdown messages.
    let want_shutdown = &[
        "shutting down the synchronous actor",
        "shutting down the thread local actor",
        "shutting down the thread safe actor",
    ];
    let output = read_output(child);
    unordered_output(want_shutdown, output);
}

#[test]
fn test_5_my_ip() {
    let mut child = run_example("5_my_ip");

    // First read the startup message, ensuring the runtime has time to start
    // up.
    let expected =
        "lvl=\"INFO\" msg=\"listening on 127.0.0.1:7890\" target=\"5_my_ip\" module=\"5_my_ip\"\n";
    let mut output = vec![0; expected.len() + 1];
    #[rustfmt::skip]
    let n = child.inner.stderr.as_mut().unwrap().read(&mut output).unwrap();
    let got = std::str::from_utf8(&output[..n]).unwrap();
    assert_eq!(expected, got);
    assert_eq!(n, expected.len());

    // Connect to the running example.
    let address = "127.0.0.1:7890".parse().unwrap();
    let mut stream = tcp_retry_connect(address);
    let mut output = String::new();
    stream
        .read_to_string(&mut output)
        .expect("unable to to read from stream");
    assert_eq!(output, "127.0.0.1");

    if let Err(err) = send_signal(child.inner.id(), libc::SIGINT) {
        panic!("unexpected error sending signal to process: {err}");
    }
}

#[test]
fn test_6_restart_supervisor() {
    let output = run_example_output("6_restart_supervisor");
    let expected = &[
        "lvl=\"WARN\" msg=\"print_actor failed, restarting it (0/2 restarts left): can't print message 'Hello world!'\" target=\"6_restart_supervisor\" module=\"6_restart_supervisor\"",
        "lvl=\"WARN\" msg=\"print_actor failed, restarting it (1/2 restarts left): can't print message 'Hello world!'\" target=\"6_restart_supervisor\" module=\"6_restart_supervisor\"",
        "lvl=\"WARN\" msg=\"print_actor failed, stopping it (no restarts left): can't print message 'Hello world!'\" target=\"6_restart_supervisor\" module=\"6_restart_supervisor\"",
        "lvl=\"WARN\" msg=\"sync_print_actor failed, restarting it (0/2 restarts left): can't print message synchronously 'Hello world!'\" target=\"6_restart_supervisor\" module=\"6_restart_supervisor\"",
        "lvl=\"WARN\" msg=\"sync_print_actor failed, restarting it (1/2 restarts left): can't print message synchronously 'Hello world!'\" target=\"6_restart_supervisor\" module=\"6_restart_supervisor\"",
        "lvl=\"WARN\" msg=\"sync_print_actor failed, stopping it (no restarts left): can't print message synchronously 'Hello world!'\" target=\"6_restart_supervisor\" module=\"6_restart_supervisor\"",
    ];
    unordered_output(expected, output);
}

/// Wrapper around a `command::Child` that kills the process when dropped, even
/// if the test failed. Sometimes the child command would survive the test when
/// running then in a loop (e.g. with `cargo watch`). This caused problems when
/// trying to bind to the same port again.
#[derive(Debug)]
struct ChildCommand {
    inner: Child,
}

impl Deref for ChildCommand {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.inner
    }
}

impl DerefMut for ChildCommand {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.inner
    }
}

impl Drop for ChildCommand {
    fn drop(&mut self) {
        let _ = self.inner.kill();
        self.inner.wait().expect("can't wait on child process");
    }
}

/// Run an example and return it's output.
fn run_example_output(name: &'static str) -> String {
    let child = run_example(name);
    read_output(child)
}

/// Run an already build example
fn run_example(name: &'static str) -> ChildCommand {
    let paths = [
        format!("../target/debug/examples/{name}"),
        // NOTE: this is not great. These target triples should really comes
        // from rustc/cargo, but this works for now.
        #[cfg(target_os = "macos")]
        format!("../target/x86_64-apple-darwin/debug/examples/{name}"),
        #[cfg(target_os = "linux")]
        format!("../target/x86_64-unknown-linux-gnu/debug/examples/{name}"),
        #[cfg(target_os = "freebsd")]
        format!("../target/x86_64-unknown-freebsd/debug/examples/{name}"),
    ];

    let mut errs = Vec::new();
    let res = Command::new("cargo")
        .args(["build", "--example", name])
        .stdin(Stdio::null())
        .output();
    match res {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let out = String::from_utf8_lossy(&output.stdout).to_owned();
            let err = String::from_utf8_lossy(&output.stdout).to_owned();
            let msg = format!("failed to build example:\n{out}\n{err}");
            errs.push(io::Error::new(io::ErrorKind::Other, msg));
        }
        Err(err) => errs.push(err),
    }

    for path in paths.iter() {
        let res = Command::new(path)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map(|inner| ChildCommand { inner });
        match res {
            Ok(cmd) => return cmd,
            Err(err) => errs.push(err),
        }
    }

    panic!("failed to run example '{name}': errors: {errs:?}");
}

/// Read the standard output of the child command.
fn read_output(mut child: ChildCommand) -> String {
    child.wait().expect("error running example");

    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut output = String::new();
    stdout
        .read_to_string(&mut output)
        .expect("error reading standard output of example");
    stderr
        .read_to_string(&mut output)
        .expect("error reading standard error of example");
    output
}

/// Expected all lines in `expected` to be in `output`.
fn unordered_output(expected: &[&str], output: String) {
    assert!(expected.is_sorted());

    // Because the order in which the actors are run is unspecified we don't
    // know the order of the output.
    let mut got: Vec<&str> = output.lines().collect();
    got.sort_unstable();
    assert_eq!(got, expected);
}

fn tcp_retry_connect(address: SocketAddr) -> TcpStream {
    for _ in 0..10 {
        if let Ok(stream) = TcpStream::connect(address) {
            return stream;
        }
        sleep(Duration::from_millis(10));
    }
    panic!("failed to connect to address");
}
