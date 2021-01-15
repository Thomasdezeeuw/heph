//! These tests perform an end-to-end test based on the examples in the examples
//! directory.

use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::ops::{Deref, DerefMut};
use std::panic;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use mio_signals::{send_signal, Signal};

#[test]
fn test_1_hello_world() {
    let output = run_example_output("1_hello_world");
    assert_eq!(output, "Hello World\n");
}

#[test]
fn test_2_my_ip() {
    let _child = run_example("2_my_ip");

    let address: SocketAddr = "127.0.0.1:7890".parse().unwrap();
    let mut stream = tcp_retry_connect(address);

    let mut output = String::new();
    stream
        .read_to_string(&mut output)
        .expect("unable to to read from stream");
    assert_eq!(output, "127.0.0.1");
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
fn test_4_sync_actor() {
    let output = run_example_output("4_sync_actor");
    assert_eq!(output, "Got a message: Hello world\nBye\n");
}

#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "multi-threaded process signal handling is broken on Linux; tracked in #325"
)]
fn test_6_process_signals() {
    let child = run_example("6_process_signals");
    // Give the process some time to setup signal handling.
    sleep(Duration::from_millis(300));

    if let Err(err) = send_signal(child.inner.id(), Signal::Interrupt) {
        panic!("unexpected error sending signal to process: {}", err);
    }

    // Because the order in which the actors are run is not defined we don't
    // know the order of the output. We do know that the greeting messages
    // come before the shutdown messages.
    let output = read_output(child);
    let mut lines = output.lines();
    // Greeting messages.
    let mut got_greetings: Vec<&str> = (&mut lines).take(3).collect();
    got_greetings.sort_unstable();
    let want_greetings = &[
        "Got a message: Hello sync actor",
        "Got a message: Hello thread local actor",
        "Got a message: Hello thread safe actor",
    ];
    assert_eq!(got_greetings, want_greetings);

    // Shutdown messages.
    let mut got_shutdown: Vec<&str> = lines.collect();
    got_shutdown.sort_unstable();
    let want_shutdown = [
        "shutting down the synchronous actor",
        "shutting down the thread local actor",
        "shutting down the thread safe actor",
    ];
    assert_eq!(got_shutdown, want_shutdown);
}

#[test]
fn test_7_restart_supervisor() {
    let output = run_example_output("7_restart_supervisor");
    let mut lines = output.lines();

    // Example timestamp "2020-08-05T13:51:53.687353Z ".
    const TIMESTAMP_OFFSET: usize = 28;
    // Index of the "?" in the string below.
    const LEFT_INDEX: usize = 64;

    let mut expected = "[WARN] 7_restart_supervisor: print actor failed, restarting it (?/5 restarts left): can't print message synchronously 'Hello world!': actor message 'Hello world!'".to_owned();
    for left in (0..5).rev() {
        let line = lines.next().unwrap();
        let line = &line[TIMESTAMP_OFFSET..];

        unsafe {
            expected.as_bytes_mut()[LEFT_INDEX] = b'0' + left;
        }
        assert_eq!(line, expected);
    }

    let expected = "[WARN] 7_restart_supervisor: print actor failed, stopping it (no restarts left): can't print message synchronously 'Hello world!': actor message 'Hello world!'";
    let last_line = lines.next().unwrap();
    let last_line = &last_line[TIMESTAMP_OFFSET..];
    assert_eq!(last_line, expected);

    let mut expected = "[WARN] 7_restart_supervisor: print actor failed, restarting it (?/5 restarts left): can't print message 'Hello world!': actor message 'Hello world!'".to_owned();
    for left in (0..5).rev() {
        let line = lines.next().unwrap();
        let line = &line[TIMESTAMP_OFFSET..];

        unsafe {
            expected.as_bytes_mut()[LEFT_INDEX] = b'0' + left;
        }
        assert_eq!(line, expected);
    }

    let expected = "[WARN] 7_restart_supervisor: print actor failed, stopping it (no restarts left): can't print message 'Hello world!': actor message 'Hello world!'";
    let last_line = lines.next().unwrap();
    let last_line = &last_line[TIMESTAMP_OFFSET..];
    assert_eq!(last_line, expected);

    // Expect no more output.
    assert_eq!(lines.next(), None);
}

/// Wrapper around a `command::Child` that kills the process when dropped, even
/// if the test failed. Sometimes the child command would survive the test when
/// running then in a loop (e.g. with `cargo watch`). This caused problems when
/// trying to bind to the same port again.
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
    Command::new(format!("target/debug/examples/{}", name))
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map(|inner| ChildCommand { inner })
        .expect("unable to run example")
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

fn tcp_retry_connect(address: SocketAddr) -> TcpStream {
    for _ in 0..10 {
        if let Ok(stream) = TcpStream::connect(address) {
            return stream;
        }
        sleep(Duration::from_millis(50));
    }
    panic!("failed to connect to address");
}
