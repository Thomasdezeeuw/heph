# 0.4.0

In this release the runtime was removed from the Heph crate and moved into the
new heph-rt crate. To see the changes made to the runtime see [Heph-rt change
log] in the rt directory.

[Heph-rt change log]: ./rt/CHANGELOG.md

## Added

* `ActorFuture` type to start an actor implementing a `Future`
  (https://github.com/Thomasdezeeuw/heph/commit/1fa9e0d5f80ceee84e93e6de0290cf842c2485e2).
* `actor::spawn_sync_actor` to spawn synchronous actors, leaving the caller in
  control of the spawned thread runnign the synchronous actor
  (https://github.com/Thomasdezeeuw/heph/commit/48ae0fbc5cf8e8d48f318baabdeb4b0cc5dd7ecf).
* `ActorRef::map_fn` and `ActorRef::try_map_fn` to create a mapped actor
  reference using a function.
  (https://github.com/Thomasdezeeuw/heph/commit/aa6ccf79de85c0abccb6a26f920c7d9d405e80cf).
* `supervisor::StopSupervisor` logs the error and stops the actor
  (https://github.com/Thomasdezeeuw/heph/commit/32ebc015a8f69be4c4f1c43ac020675e3c96cc09).

## Changed

* Changed the API of `NewActor::name` to be an associated method
  (https://github.com/Thomasdezeeuw/heph/commit/099ddd19a0d087577a3dc84cfec8ddb8d572a05b).
* The `messages` module moved to the root of crate from `actor::message`
  (https://github.com/Thomasdezeeuw/heph/commit/68de95431abca0aa33704e64364c928df5c5152c).
* Added `RuntimeAccess` type to the `SyncActor` trait, similar to
  `NewActor::RuntimeAccess`
  (https://github.com/Thomasdezeeuw/heph/commit/33f0f485f1fbd9bd0ece76dce12040232fea2bc7).
* Now using key-value logging capability of `log` crate
  (https://github.com/Thomasdezeeuw/heph/commit/cb58771cb380aee601a733f2f66a9ada9998814e,
  https://github.com/Thomasdezeeuw/heph/commit/0e0b0d16c11b4ed93e0f02982545ef6eff0977d4).
* Two metrics, printed on receiving the `SIGUSR2` signal, have changed names
  (https://github.com/Thomasdezeeuw/heph/commit/0f81654c63ca65a8c4ffb7d246cd7a219f98b637):
  * `os` -> `host_os`.
  * `architecture` -> `host_arch`.
* Reduced the allocations made joining a mapped `ActorRef`
  (https://github.com/Thomasdezeeuw/heph/commit/f0d8fc7b555fbda6d88e15032503e9516e103241).
* Reduced the allocations made sending with a mapped `ActorRef`
  (https://github.com/Thomasdezeeuw/heph/commit/80baf5b4e5d256372911141710a130d19dd8a5d4).
* Switch to Rust edition 2021
  (https://github.com/Thomasdezeeuw/heph/commit/3a6c206946e656dda344da5ca36a9b35e50abed3).

## Removed

* The `rt` module has been removed and now lives on as the `heph-rt` crate.
  Along with that the `Runtime` and `RuntimeRef` aliases are removed from the
  root of crate.
* Furthermore the `log`, `net`, `spawn`, `timer` and `trace` modules also has
  been moved to the `heph-rt` crate.
* A lot of functions in the `test` module were moved to the `heph-rt` crate.

# 0.3.1

## Added

* `ActorRef::join`
  (https://github.com/Thomasdezeeuw/heph/commit/2987992735164fd2fd3c8f7a1dd681f13b14ccbb).
* `From<ActorRef>` implementation for `ActorGroup`
  (https://github.com/Thomasdezeeuw/heph/commit/75dff2b1968c3f850c4332d802362d7421c15e5e).
* `ActorGroup::join_all`
  (https://github.com/Thomasdezeeuw/heph/commit/50e4da5d1ff91ce46f323aed4b5a8ce3e92dbd0f).
* `rt::Signal::User1` and `Signal::User2`
  (https://github.com/Thomasdezeeuw/heph/commit/c039c92d4415125b25878313505f5d4d12f960e8).
* `net::Bytes::limit`
  (https://github.com/Thomasdezeeuw/heph/commit/7c2b12db4a7845b0bfb33dd7d1325e95aff8236f).
* `net::BytesVectored::limit`
  (https://github.com/Thomasdezeeuw/heph/commit/35551fb652462c3a7343dc3ca79004c969eb2288).
* `test::try_spawn_local`
  (https://github.com/Thomasdezeeuw/heph/commit/f3cd71cb09b41e21af2c0bcc1091bad7b5d651eb).
* `test::try_spawn`
  (https://github.com/Thomasdezeeuw/heph/commit/5a086b0ca2f358133c4cd7884adadbf15f180329).
* `test::spawn_future`
  (https://github.com/Thomasdezeeuw/heph/commit/3f58f18e49e3b925ff5c6f04fa9f8b827aa065ff).
* `test::PanicSupervisor`
  (https://github.com/Thomasdezeeuw/heph/commit/9fa67d58de15cc207eb3ec48444d0dc9ed0eabea).
* `test::join` and `test::join_many`
  (https://github.com/Thomasdezeeuw/heph/commit/1a1bab05260ba0640ca5bcb2fbbcc69ac97daab8).
* `test::join_all`
  (https://github.com/Thomasdezeeuw/heph/commit/e78fb7f4fe3f2e3843009ab42b5f77ee339d86e8).
* *EXPERIMENTAL*: output runtime metrics on `SIGUSR2` process signal
  (https://github.com/Thomasdezeeuw/heph/commit/f451006939c5a76ad51e353d7c9f194e640dfe46).
  The metrics *currently* include the following:
   * Heph version
   * OS (version)
   * Architecture
   * Hostname
   * Host id
   * Application name
   * Process id
   * Parent process id
   * Uptime
   * Number of worker threads
   * Number of synchronous actors
   * Scheduler metrics, including number of inactive and ready actors
   * Number of timers and next timer ready
   * What process signals we're listening to
   * Number of process signal receivers (also see per worker thread)
   * Total CPU time
   * CPU time
   * Trace log: file path and number of traces
   * Per worker thread:
      * Worker id
      * Scheduler metrics, including number of inactive and ready actors
      * Number of timers and next timer ready
      * Number of process signal receivers
      * CPU affinity
      * CPU time
      * Trace log: number of traces

## Changed

* Don't return an error when failing to relay a process signal to a worker
  thread
  (https://github.com/Thomasdezeeuw/heph/commit/0c46f9d2e7e87b58df196dbc3e82e022235b2afc).
* Updated rustc version to work on the nightly of at least 2021-08-13.

## Fixed

* Memory leak in scheduler
  (https://github.com/Thomasdezeeuw/heph/commit/36dc37e0ada1911a3f4399767997a24c4a42441c).
* Send the correct number of bytes in `TcpStream::send_file_all`
  (https://github.com/Thomasdezeeuw/heph/commit/ea62b8d49657c3e7fb49159f1285ba33b5f7ffbd).
* Data race in RPC
  (https://github.com/Thomasdezeeuw/heph/commit/76ecb6a50afda975f10c59973601fb45919de351,
   https://github.com/Thomasdezeeuw/inbox/commit/2dd49a96e55e97e66a6634eab92cb81764c8d0cdk).
* Don't ignore EINVAL on macOS in `TcpStream::connect`
  (https://github.com/Thomasdezeeuw/heph/commit/7b7d4bde5db2624de92d660a6b74416e6dab7381).

# 0.3.0

Initial public release.
