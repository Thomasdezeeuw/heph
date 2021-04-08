# 0.1.1

## Added

* Support of ThreadSanitizer, by using atomic loads instead of fences when the
  thread sanitizer is enabled.
  (https://github.com/Thomasdezeeuw/inbox/commit/dc5202e0d621856403a125dcef5bf33e9477d2c4).

## Changed

* Optimised `oneshot::Receiver::try_reset` by dropping the message in place on
  the heap.
  (https://github.com/Thomasdezeeuw/inbox/commit/5a5b5421869e152c18436d56470dd4c4619317e8).

## Fixes

* Data race when dropping `oneshot::Sender`/`Receiver`
  (https://github.com/Thomasdezeeuw/inbox/commit/96ac5a09e2161aebf3a63ff099272828ba961492).
* Possible data race in oneshot channel
  (https://github.com/Thomasdezeeuw/inbox/commit/d40f7db267aabffc446ff6e9860f03f0fc6aa92d).

To catch these kind of problems we now run the address, leak, memory and thread
sanitizers on the CI, in addition to Miri (which we already ran)
(https://github.com/Thomasdezeeuw/inbox/commit/b45ce7ebac8b2926b07a3670276d5548a0426e8a).

# 0.1.0

Initial release.
