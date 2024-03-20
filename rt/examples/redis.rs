//! This example provides are redis-like server implementation. It implements
//! the following commands:
//! * GET
//! * SET

#![feature(never_type)]

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use heph::actor::{self, actor_fn, Actor, NewActor};
use heph::supervisor::{Supervisor, SupervisorStrategy};
use heph_rt::net::{tcp, TcpStream};
use heph_rt::spawn::options::{ActorOptions, Priority};
use heph_rt::timer::Deadline;
use heph_rt::{self as rt, Runtime};
use log::{error, info};
use std_logger::request;

/// Error string that can be returned, following RESP (i.e. starts with `-` and
/// ends with `\r\n`).
type Err = &'static str;

// We support two commands: GET and SET.
const COMMANDS: &str = "*2\r\n\
                        *6\r\n$3\r\nget\r\n:2\r\n*2\r\n+readonly\r\n+fast\r\n:1\r\n:1\r\n:1\r\n\
                        *6\r\n$3\r\nset\r\n:3\r\n*2\r\n+write\r\n+denyoom\r\n:1\r\n:1\r\n:1\r\n";
const NIL: &str = "$-1\r\n";
const OK: &str = "+OK\r\n";
const ERR_UNIMPLEMENTED: &str = "-not implemented\r\n";
const ERR_INVALID_ARGUMENTS: &str = "-invalid number of arguments\r\n";
const ERR_PARSE_INT: &str = "-unable to parse integer\r\n";
const ERR_PARSE_STR: &str = "-unable to parse string\r\n";
const ERR_PARSE_ENDLINE: &str = "-unable to parse end of line\r\n";

const TIMEOUT: Duration = Duration::from_secs(10);

fn main() -> Result<(), rt::Error> {
    // Enable logging.
    std_logger::Config::logfmt().init();

    let values = Arc::new(RwLock::new(HashMap::new()));
    let actor = actor_fn(conn_actor).map_arg(move |stream| (stream, values.clone()));
    let address = "127.0.0.1:6379".parse().unwrap();
    let server = tcp::server::setup(address, conn_supervisor, actor, ActorOptions::default())
        .map_err(rt::Error::setup)?;

    let mut runtime = Runtime::setup().use_all_cores().build()?;
    runtime.run_on_workers(move |mut runtime_ref| -> io::Result<()> {
        let options = ActorOptions::default().with_priority(Priority::LOW);
        let server_ref = runtime_ref.spawn_local(ServerSupervisor, server, (), options);

        runtime_ref.receive_signals(server_ref.try_map());
        Ok(())
    })?;
    info!("listening on {address}");
    runtime.start()
}

#[derive(Copy, Clone, Debug)]
struct ServerSupervisor;

impl<NA> Supervisor<NA> for ServerSupervisor
where
    NA: NewActor<Argument = (), Error = !>,
    NA::Actor: Actor<Error = tcp::server::Error<!>>,
{
    fn decide(&mut self, err: tcp::server::Error<!>) -> SupervisorStrategy<()> {
        use tcp::server::Error::*;
        match err {
            Accept(err) => {
                error!("error accepting new connection: {err}");
                SupervisorStrategy::Restart(())
            }
            NewActor(_) => unreachable!(),
        }
    }

    fn decide_on_restart_error(&mut self, err: !) -> SupervisorStrategy<()> {
        err
    }

    fn second_restart_error(&mut self, err: !) {
        err
    }
}

fn conn_supervisor(err: io::Error) -> SupervisorStrategy<TcpStream> {
    error!("error handling connection: {err}");
    SupervisorStrategy::Stop
}

async fn conn_actor<RT>(
    ctx: actor::Context<!, RT>,
    stream: TcpStream,
    values: Arc<RwLock<HashMap<Box<str>, Arc<[u8]>>>>,
) -> io::Result<()>
where
    RT: rt::Access + Clone,
{
    let address = stream.peer_addr()?;
    info!(address:% = address; "accepted connection");
    let mut buffer = Vec::with_capacity(1024);

    let err = loop {
        buffer.clear();
        buffer = Deadline::after(ctx.runtime_ref().clone(), TIMEOUT, stream.recv(buffer)).await?;
        if buffer.is_empty() {
            return Ok(());
        }
        let buf = &buffer[..];

        // A `GET key` commands look like:
        // *2\r\n$3\r\nGET\r\n$3\r\nkey\r\n
        // A `SET key value`:
        // *3\r\n$3\r\nSET\r\n$3\r\nkey\r\n$5\r\nvalue\r\n
        match buf[0] {
            b'*' => {
                // * starts an Array.
                // Parse the length of the array.
                let (length, n) = match parse_int(&buf[1..]) {
                    Ok(res) => res,
                    Err(err) => break err,
                };
                let buf = &buf[1 + n..];
                // The first argument should always be the command as string.
                // NOTE: in production don't make the assumption above.
                let (cmd, n) = match parse_str(buf) {
                    Ok(res) => res,
                    Err(err) => break err,
                };
                let buf = &buf[n..];
                request!("'{cmd}' command");
                match cmd {
                    "GET" => {
                        if length != 2 {
                            break ERR_INVALID_ARGUMENTS;
                        }
                        // The only argument to the command should be the key.
                        let (key, n) = match parse_str(buf) {
                            Ok(res) => res,
                            Err(err) => break err,
                        };
                        let buf = &buf[n..];
                        if !buf.is_empty() {
                            // The key should be the end of the command.
                            // We don't support pipelined commands in this
                            // example.
                            break ERR_UNIMPLEMENTED;
                        }

                        let value = { values.read().unwrap().get(key).cloned() };
                        buffer.clear();
                        if let Some(value) = value {
                            write!(&mut buffer, "${}\r\n", value.len()).unwrap();
                            let bufs = (buffer, value, "\r\n");
                            let bufs = stream.send_vectored_all(bufs).await?;
                            buffer = bufs.0;
                        } else {
                            stream.send_all(NIL).await?;
                        }
                    }
                    "SET" => {
                        if length != 3 {
                            break ERR_INVALID_ARGUMENTS;
                        }
                        // The only argument to the command should be the key.
                        let (key, n) = match parse_str(buf) {
                            Ok(res) => res,
                            Err(err) => break err,
                        };
                        let buf = &buf[n..];
                        let (value, n) = match parse_bytes(buf) {
                            Ok(res) => res,
                            Err(err) => break err,
                        };
                        let buf = &buf[n..];
                        if !buf.is_empty() {
                            // The value should be the end of the command.
                            // We don't support pipelined commands in this
                            // example.
                            break ERR_UNIMPLEMENTED;
                        }

                        let key = Box::from(key);
                        let value = Arc::from(value);
                        {
                            values.write().unwrap().insert(key, value);
                        }
                        stream.send_all(OK.as_bytes()).await?;
                    }
                    "COMMAND" => {
                        stream.send_all(COMMANDS).await?;
                    }
                    _ => break ERR_UNIMPLEMENTED,
                }
            }
            _ => break ERR_UNIMPLEMENTED,
        }
    };
    stream.send_all(err).await?;
    Ok(())
}

/// Parse an integer from `buf` including `\r\n`.
/// Returns (value, bytes_read).
fn parse_int(buf: &[u8]) -> Result<(usize, usize), Err> {
    let mut value: usize = 0;
    let mut n = 0;
    let mut bytes = buf.into_iter();
    while let Some(b) = bytes.next() {
        match b {
            b'0'..=b'9' => {
                match value.checked_add((b - b'0') as usize) {
                    Some(v) => value = v,
                    None => return Err(ERR_PARSE_INT),
                }
                n += 1;
            }
            b'\r' => match bytes.next() {
                Some(b'\n') => return Ok((value, n + 2)),
                _ => return Err(ERR_PARSE_ENDLINE),
            },
            _ => return Err(ERR_PARSE_INT),
        }
    }
    // NOTE: we should actually read more bytes, but for sake of an example
    // we're not doing that.
    Err(ERR_PARSE_INT)
}

/// Parse a string from `buf` including `\r\n`.
fn parse_str<'a>(buf: &'a [u8]) -> Result<(&'a str, usize), Err> {
    let (string, n) = parse_bytes(buf)?;
    match std::str::from_utf8(string) {
        Ok(string) => Ok((string, n)),
        Err(_) => Err(ERR_PARSE_STR),
    }
}

/// Parse a byte string from `buf` including `\r\n`.
fn parse_bytes<'a>(buf: &'a [u8]) -> Result<(&'a [u8], usize), Err> {
    if !matches!(buf.first(), Some(b'$')) {
        return Err(ERR_PARSE_STR);
    }
    let (length, n) = parse_int(&buf[1..])?;

    let buf = &buf[1 + n..];
    if buf.len() < length + 2 {
        return Err(ERR_PARSE_STR);
    }

    if !matches!(buf.get(length), Some(b'\r')) || !matches!(buf.get(length + 1), Some(b'\n')) {
        return Err(ERR_PARSE_STR);
    }

    Ok((&buf[..length], 1 + n + length + 2))
}
