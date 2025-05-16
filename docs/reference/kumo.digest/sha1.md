# sha1

```lua
kumo.digest.sha1(ARGS)
```

Computes a SHA1 digest over each of the arguments in ARGS.

!!!note
    SHA1 is a deprecated algorithm that is no longer recommended
    for cryptographic usage.

You may pass multiple arguments.

String arguments are intepreted as bytes and fed into the digest algorithm.

Other types are first encoded as a JSON string and that string is then fed
into the digest algorithm.

The returned value is a [Digest](index.md) object representing the digest
bytes. It has properties that can return the digest bytes or encoded in
a number of common and useful encodings.
