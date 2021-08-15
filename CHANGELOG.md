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
