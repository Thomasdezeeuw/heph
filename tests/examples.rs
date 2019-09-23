//! These tests perform an end-to-end test based on the examples in the examples
//! directory.

use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::ops::{Deref, DerefMut};
use std::panic;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

use lazy_static::lazy_static;

// The tests need to run in sequentially here because Cargo can only build one
// example at a time. This would cause problems with the example that don't wait
// for the full execution of the example, this is for example the case in
// example 2 which needs to send a request to the running example.

/// Macro to create a group of sequential tests.
macro_rules! sequential_tests {
    ($(fn $name: ident () $body: block)+) => {
        lazy_static! {
            /// A global lock for testing sequentially.
            static ref SEQUENTIAL_TESTS: Mutex<()> = Mutex::new(());
        }

        $(
        #[test]
        fn $name() {
            let guard = SEQUENTIAL_TESTS.lock().unwrap();
            // Catch any panics to not poison the lock.
            if let Err(err) = panic::catch_unwind(|| $body) {
                drop(guard);
                panic::resume_unwind(err);
            }
        }
        )+
    };
}

sequential_tests! {
    fn test_1_hello_world() {
        let output = run_example_output("1_hello_world");
        assert_eq!(output, "Hello World\n");
    }

    fn test_1b_hello_world_() {
        let output = run_example_output("1b_hello_world");
        assert_eq!(output, "Hello World\n");
    }

    fn test_2_my_ip() {
        let _child = run_example("2_my_ip");

        let address: SocketAddr = "127.0.0.1:7890".parse().unwrap();
        let mut stream = tcp_retry_connect(address);

        let mut output = String::new();
        stream.read_to_string(&mut output)
            .expect("unable to to read from stream");
        assert_eq!(output, "127.0.0.1");
    }
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
    build_example(name);
    let child = start_example(name);
    read_output(child)
}

/// Run an example, not waiting for it to complete, but it does wait for it to
/// be build.
fn run_example(name: &'static str) -> ChildCommand {
    build_example(name);
    start_example(name)
}

/// Build the example with the given name.
fn build_example(name: &'static str) {
    let output = Command::new("cargo")
        .args(&["build", "--example", name])
        .output()
        .expect("unable to build example");

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("failed to build example: {}\n\n{}", stdout, stderr);
    }
}

/// Start and already build example
fn start_example(name: &'static str) -> ChildCommand {
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
    let mut output = String::new();
    stdout
        .read_to_string(&mut output)
        .expect("error reading output of example");
    output
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
