//! These test perform an end-to-end test based on the examples in the examples
//! directory.

use std::io::{Read, Write};
use std::net::{TcpStream, SocketAddr};
use std::process::{Command, Child, Stdio};
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;
use std::panic;

use lazy_static::lazy_static;

// The tests need to run in sequentially here because Cargo can only build one
// example at a time. This would cause problems with the example that don't wait
// for the full execution of the example, this is for example the case in
// example 2 which needs to send a request to the running example.

lazy_static! {
    /// A global lock for testing sequentially, see `sequential_test` macro.
    static ref SEQUENTIAL_TEST_MUTEX: Mutex<()> = Mutex::new(());
}

/// Macro to crate a sequential test, that locks `SEQUENTIAL_TEST_MUTEX` while
/// testing.
macro_rules! sequential_test {
    (fn $name:ident() $body:block) => {
        #[test]
        fn $name() {
            let guard = SEQUENTIAL_TEST_MUTEX.lock().unwrap();
            // Catch any panics to not poison the lock.
            if let Err(err) = panic::catch_unwind(|| $body) {
                drop(guard);
                panic::resume_unwind(err);
            }
        }
    };
}

sequential_test! {
    fn example_1_hello_world() {
        let child = run_example("1_hello_world");

        let output = read_output(child);
        assert_eq!(output, "Hello World\n");
    }
}

sequential_test! {
    fn example_2_my_ip() {
        let mut child = run_example("2_my_ip");
        build_time_sleep();

        let address: SocketAddr = "127.0.0.1:7890".parse().unwrap();
        let mut stream = TcpStream::connect(address).expect("unable to connect");

        let mut output = String::new();
        stream.read_to_string(&mut output)
            .expect("unable to to read from stream");
        assert_eq!(output, "127.0.0.1");

        child.kill().expect("can't kill child process");
        child.wait().unwrap();
    }
}

sequential_test! {
    fn example_3_echo_server() {
        let mut child = run_example("3_echo_server");
        build_time_sleep();

        let address: SocketAddr = "127.0.0.1:7890".parse().unwrap();
        let mut stream = TcpStream::connect(address).expect("unable to connect");

        const SEND: &'static [u8] = b"Hello World";
        stream.write(SEND).unwrap();

        let mut output = [0; 11];
        stream.read(&mut output).unwrap();
        assert_eq!(output, SEND, "should be `Hello World`");

        child.kill().expect("can't kill child process");
        child.wait().unwrap();
    }
}

/// Run the example with the given name.
fn run_example(name: &'static str) -> Child {
    Command::new("cargo")
        .args(&["run", "--example", name])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("unable to run example")
}

/// Read the standard output of the child command.
fn read_output(mut child: Child) -> String {
    child.wait().expect("error running example");

    let mut stdout = child.stdout.take().unwrap();
    let mut output = String::new();
    stdout.read_to_string(&mut output).expect("error reading output of example");
    output
}

/// Sleep for a while to give Cargo the time to build and run the example.
fn build_time_sleep() {
    // Give the program some time to build and start serving requests.
    sleep(Duration::from_millis(500));
}
