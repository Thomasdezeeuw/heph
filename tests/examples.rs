//! These tests perform an end-to-end test based on the examples in the examples
//! directory.

use std::io::{self, Read};
use std::ops::{Deref, DerefMut};
use std::panic;
use std::process::{Child, Command, Stdio};

#[test]
fn test_1_hello_world() {
    let output = run_example_output("1_hello_world");
    assert_eq!(output, "Hello World\n");
}

#[test]
fn test_2_rpc() {
    let output = run_example_output("2_rpc");
    assert_eq!(
        output,
        "Got a RPC request: Ping\nGot a RPC response: Pong\n"
    );
}

#[test]
fn test_3_sync_actor() {
    let output = run_example_output("3_sync_actor");
    assert_eq!(output, "Got a message: Hello world\nBye\n");
}

#[test]
#[ignore]
fn test_4_restart_supervisor() {
    // Index of the "?" in the string below.
    const LEFT_INDEX: usize = 51;

    let output = run_example_output("4_restart_supervisor");
    let mut lines = output.lines();

    let mut expected = "lvl=\"WARN\" msg=\"print actor failed, restarting it (?/5 restarts left): can't print message synchronously 'Hello world!': actor message 'Hello world!'\" target=\"restart_supervisor\" module=\"restart_supervisor\"".to_owned();
    for left in (0..5).rev() {
        let line = lines.next().unwrap();

        unsafe {
            expected.as_bytes_mut()[LEFT_INDEX] = b'0' + left;
        }
        assert_eq!(line, expected);
    }

    let expected = "lvl=\"WARN\" msg=\"print actor failed, stopping it (no restarts left): can't print message synchronously 'Hello world!': actor message 'Hello world!'\" target=\"restart_supervisor\" module=\"restart_supervisor\"";
    let last_line = lines.next().unwrap();
    assert_eq!(last_line, expected);

    let mut expected = "lvl=\"WARN\" msg=\"print actor failed, restarting it (?/5 restarts left): can't print message 'Hello world!': actor message 'Hello world!'\" target=\"restart_supervisor\" module=\"restart_supervisor\"".to_owned();
    for left in (0..5).rev() {
        let line = lines.next().unwrap();

        unsafe {
            expected.as_bytes_mut()[LEFT_INDEX] = b'0' + left;
        }
        assert_eq!(line, expected);
    }

    let expected = "lvl=\"WARN\" msg=\"print actor failed, stopping it (no restarts left): can't print message 'Hello world!': actor message 'Hello world!'\" target=\"restart_supervisor\" module=\"restart_supervisor\"";
    let last_line = lines.next().unwrap();
    assert_eq!(last_line, expected);

    // Expect no more output.
    assert_eq!(lines.next(), None);
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
        format!("target/debug/examples/{name}"),
        // NOTE: this is not great. These target triples should really comes
        // from rustc/cargo, but this works for now.
        #[cfg(target_os = "macos")]
        format!("target/x86_64-apple-darwin/debug/examples/{name}"),
        #[cfg(target_os = "linux")]
        format!("target/x86_64-unknown-linux-gnu/debug/examples/{name}"),
        #[cfg(target_os = "freebsd")]
        format!("target/x86_64-unknown-freebsd/debug/examples/{name}"),
    ];

    let mut errs = Vec::new();
    loop {
        for path in paths.iter() {
            let res = Command::new(path)
                .stdin(Stdio::null())
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .map(|inner| ChildCommand { inner });
            match res {
                Ok(cmd) => return cmd,
                Err(ref err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => errs.push(err),
            }
        }

        if errs.is_empty() {
            let res = Command::new("cargo")
                .args(["build", "--example", name])
                .stdin(Stdio::null())
                .output();
            match res {
                Ok(output) if output.status.success() => continue,
                Ok(output) => {
                    let out = String::from_utf8_lossy(&output.stdout).to_owned();
                    let err = String::from_utf8_lossy(&output.stdout).to_owned();
                    let msg = format!("failed to build example:\n{out}\n{err}");
                    errs.push(io::Error::new(io::ErrorKind::Other, msg));
                }
                Err(err) => errs.push(err),
            }
        }
        break;
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
