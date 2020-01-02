# Remote Message Passing

Current ideas for remote message passing for Heph.

The communication must be encrypted by default to protect the messages from
being tempered with.

Since the guarantees provided for sending messages don't include guaranteed
delivery we can get away with using UDP for the messages. This does limit the
message size to ~65k (minus encryption overhead), but this should be enough for
most messages. And if at some point it isn't enough for some use cases we can
introduce message fragmentation and send a message over multiple UDP packets
(you know reinvention part of TCP).


## Requirements

There are a number of requirement we need to satisfy to ensure proper
communication:


### Authentication

We need some form of authentication to ensure the origins of a message is the
actual sender. We don't want to process messages from an (evil) third party and
compromise our system.


### Integrity

Any message we accept must be checked for tempering from an (evil) third party.


## Possible solution

There many (partial) solution to this problem, just to name a few:

* DTLS for encrypting UDP packets/connections.
* PGP used for communication of email.
* HMAC for signing messages.
* Mosh SSH uses UDP and public/private key authentication.


### Current idea

Two way private-public key usage. Each Heph process generates its own
private-public key pair on startup, it shares it (somehow) with all other
processes, possibly via some central shared authentication checker.

When a process wants to communication with another process it requests it public
key and uses it to send it messages. It encrypts the message using the public
and then it's own private key. The receiving process requests the public key for
the source address and decrypt the message with the public key of the sender and
its own private key.
