# How Heph does actor isolation.

There are two kinds of faults: errors and failures. An error is a problem in
regular processing and should be handled via the `Result` type. It should never
bring down the system and the error should be returned to the actor's
supervisor, if it can't be handled by the actor itself.

A failure is a problem which can not be corrected and should not occur in
regular processing, for example dividing by zero. The most common form of a
failure in Rust is a `panic!`. These are often caused by programmer error (i.e.
bugs) and are hard or nearly impossible to recover from, as such Heph makes no
attempt to do so. Heph simply doesn't handle these kinds of faults. Thus if a
failure occurs the system halts as it is pointless to continue.
