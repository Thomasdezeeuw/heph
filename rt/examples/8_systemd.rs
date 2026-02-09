use std::net::{Ipv4Addr, SocketAddr};
use std::{env, io};

use heph::actor::{self, actor_fn};
use heph::restart_supervisor;
use heph::supervisor::StopSupervisor;
use heph_rt::fd::AsyncFd;
use heph_rt::net::TcpServer;
use heph_rt::spawn::options::{ActorOptions, Priority};
use heph_rt::{self as rt, Runtime, ThreadLocal};
use log::info;

// Heph has various utilties to support [systemd], thsi example shows those can
// be used. You can use the following [service] file:
//
// ```
// [Unit]
// Description=Heph Test service
// Requires=network.target
//
// [Service]
// # Required to setup communication between the service manager (systemd) and the
// # service itself.
// Type=notify
// # Restart the service if it fails.
// Restart=on-failure
// # Require the service to send a keep-alive ping every minute.
// WatchdogSec=1min
//
// # Path to the example.
// ExecStart=/path/to/heph/target/debug/examples/8_systemd
// # The port the service will listen on.
// Environment=PORT=8000
// # Enable some debug logging.
// Environment=DEBUG=1
// ```
//
// [systemd]: https://systemd.io
// [service]: https://www.freedesktop.org/software/systemd/man/systemd.service.html
fn main() -> Result<(), rt::Error> {
    // Enable logging.
    std_logger::Config::logfmt().init();

    // We should get the port from systemd, or use a default.
    let port = match env::var("PORT").as_deref().unwrap_or("7890").parse() {
        Ok(port) => port,
        Err(err) => return Err(rt::Error::setup(format!("failed to parse port: {err}"))),
    };
    let address = (Ipv4Addr::LOCALHOST, port).into();
    let actor = actor_fn(conn_actor);
    let server = TcpServer::new(address, StopSupervisor, actor, ActorOptions::default())
        .map_err(rt::Error::setup)?;

    let mut runtime = Runtime::setup()
        .use_all_cores()
        .auto_cpu_affinity()
        .build()?;

    #[cfg(target_os = "linux")]
    {
        let actor = actor_fn(heph_rt::systemd::watchdog);
        // NOTE: this should do a proper health check of you application.
        let health_check = || -> Result<(), String> { Ok(()) };
        let options = ActorOptions::default().with_priority(Priority::HIGH);
        let systemd_ref = runtime.spawn(StopSupervisor, actor, health_check, options);
        runtime.receive_signals(systemd_ref.try_map());
    }

    runtime.run_on_workers(move |mut runtime_ref| -> io::Result<()> {
        let supervisor = ServerSupervisor::new();
        let options = ActorOptions::default().with_priority(Priority::LOW);
        let server_ref = runtime_ref.spawn_local(supervisor, server, (), options);
        runtime_ref.receive_signals(server_ref.try_map());
        Ok(())
    })?;

    info!("listening on {address}");
    runtime.start()
}

restart_supervisor!(ServerSupervisor, ());

async fn conn_actor(_: actor::Context<(), ThreadLocal>, stream: AsyncFd) -> io::Result<()> {
    let address: SocketAddr = stream.peer_addr().await?;
    info!(address:%; "accepted connection");
    let ip = address.ip().to_string();
    stream.send_all(ip).await?;
    Ok(())
}
