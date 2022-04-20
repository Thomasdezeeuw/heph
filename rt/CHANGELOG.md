# 0.4.0

Initial public release. The changes below are compared to Heph v0.3.1 (last
release to include the runtime).

## Added

* The `access` module, holding the `Access` trait and the `ThreadLocal`,
  `ThreadSafe` and `Sync` implementations
  (https://github.com/Thomasdezeeuw/heph/commit/e6514071364d4ddb5b1dc9b6f1df19ee4f771eda).
* The `pipe` module was added to support Unix pipes
  (https://github.com/Thomasdezeeuw/heph/commit/0c4f1ab3eaf08bea1d65776528bfd6114c9f8374).
* The `systemd` module was added to better interface with systemd
  (https://github.com/Thomasdezeeuw/heph/commit/a97398777e9f08b9b5467ecd9aea4311cc885432 and more).
* `test::block_on`
  (https://github.com/Thomasdezeeuw/heph/commit/8f8426db6a1ae84d9ee7ea8dd6ae58ef1b6bb89f).
* `test::spawn_local_future`
  (https://github.com/Thomasdezeeuw/heph/commit/8f8426db6a1ae84d9ee7ea8dd6ae58ef1b6bb89f).

## Changed

* The `heph::rt` module is now effectively the root of the `heph-rt` crate.
* `Setup`, `ActorOptions`, `SyncActorOptions` and `FutureOptions` are marked as
  `must_use`
  (https://github.com/Thomasdezeeuw/heph/commit/c4d634f3e90691f099e9d1487721279e0d490f0f).
* `heph_rt::Config::default` was renamed to `new` to now overshadow a possible
  `Default` implementation
  (https://github.com/Thomasdezeeuw/heph/commit/d617d6018515e732df838252f6f46273f95b0d0a).
* Panics are now caught when running actors
  (https://github.com/Thomasdezeeuw/heph/commit/ad5c4150aa4f194e644c2408f2372d1174f3b3b0).
* Panics are now caught when running futures
  (https://github.com/Thomasdezeeuw/heph/commit/91774018075e899dda7887754c6f5851f58fb44d).
* Restarted actors are now scheduled instead of ran to prevent an infinite
  restart loop
  (https://github.com/Thomasdezeeuw/heph/commit/6e182f5b030bef6c3cbd82023873fd9b35656d2f).

## Fixes

* When one worker thread stops all others are awoken to ensure all worker
  threads shutdown down
  (https://github.com/Thomasdezeeuw/heph/commit/adba587bdfc6386fcda091114c7ca8dc59d7c614).
