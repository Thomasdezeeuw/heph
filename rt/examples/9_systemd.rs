#![feature(never_type)]

use std::net::{Ipv4Addr, SocketAddr};
use std::{env, io};

use heph::supervisor::StopSupervisor;
use heph::{actor, restart_supervisor};
use heph_rt::net::{TcpServer, TcpStream};
use heph_rt::spawn::options::{ActorOptions, Priority};
use heph_rt::{self as rt, Runtime, ThreadLocal};
use log::info;

fn main() -> Result<(), rt::Error> {
    std_logger::Config::logfmt().init();

    // We should get the port from systemd, or use a default.
    let port = match env::var("PORT").as_deref().unwrap_or("7890").parse() {
        Ok(port) => port,
        Err(err) => return Err(rt::Error::setup(format!("failed to parse port: {err}"))),
    };
    let address = (Ipv4Addr::LOCALHOST, port).into();
    let supervisor = StopSupervisor::for_actor("connection actor");
    let actor = conn_actor as fn(_, _, _) -> _;
    let server = TcpServer::setup(address, supervisor, actor, ActorOptions::default())
        .map_err(rt::Error::setup)?;

    let mut runtime = Runtime::setup()
        .use_all_cores()
        .auto_cpu_affinity()
        .build()?;

    #[cfg(target_os = "linux")]
    {
        let supervisor = StopSupervisor::for_actor("systemd actor");
        let actor = heph_rt::systemd::actor as fn(_, _) -> _;
        // NOTE: this should do a proper health check of you application.
        let health_check = || -> Result<(), !> { Ok(()) };
        let options = ActorOptions::default().with_priority(Priority::HIGH);
        let systemd_ref = runtime.spawn(supervisor, actor, health_check, options);
        runtime.receive_signals(systemd_ref.try_map());
    }

    runtime.run_on_workers(move |mut runtime_ref| -> io::Result<()> {
        let supervisor = ServerSupervisor::new();
        let options = ActorOptions::default().with_priority(Priority::LOW);
        let server_ref = runtime_ref.try_spawn_local(supervisor, server, (), options)?;
        runtime_ref.receive_signals(server_ref.try_map());
        Ok(())
    })?;

    info!("listening on {address}");
    runtime.start()
}

restart_supervisor!(ServerSupervisor, "TCP server actor", ());

async fn conn_actor(
    _: actor::Context<!, ThreadLocal>,
    mut stream: TcpStream,
    address: SocketAddr,
) -> io::Result<()> {
    info!("accepted connection: address={address}");
    let ip = address.ip().to_string();
    stream.send_all(ip.as_bytes()).await
}
