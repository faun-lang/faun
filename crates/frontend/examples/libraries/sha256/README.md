# sha256

SHA-256 for Faun, as FIPS 180-4 gives it: the 32-byte digest of a
message, answered for the whole message at once or fed in a piece at a
time. A `Digest` is a type of its own, read as bytes or as the 64 hex
digits `sha256sum` prints.

```ruby ignore
package {
  uses: {
    sha256: "../src/sha256"
  }
}

import sha256 {Sha256, Digest, sha256}

expect sha256("abc".to_utf8()).to_hex() == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
expect sha256([]).to_hex() == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"

# The same digest, fed in two pieces.
expect Sha256.empty.update("a".to_utf8()).update("bc".to_utf8()).digest() == sha256("abc".to_utf8())

# The bytes, most significant first, and the digest printed as its hex.
expect sha256("abc".to_utf8()).to_bytes().take(4) == [0xba, 0x78, 0x16, 0xbf]
expect "${sha256("abc".to_utf8())}" == sha256("abc".to_utf8()).to_hex()
```

Every expectation above is one of the library's own, in
[`src/sha256.fn`](src/sha256.fn), beneath the definition it shows. The
library also holds FIPS 180-4's 448-bit and 896-bit messages, and
[`crates/cli/tests/sha256.rs`](../../../../cli/tests/sha256.rs) hashes
its last, a million a's, with the examples built.

## When to use it

SHA-256 is a cryptographic hash: no one knows how to find two messages
with one digest, nor a message for a digest given. So a digest names its
message, and can stand in for it where the message is too large to keep
or compare:

- checking a download against the digest its site publishes;
- storing a file under its digest, so two copies are stored once;
- keying a cache by what an entry was made from.

It is not what to use for:

- **passwords.** SHA-256 is fast, so a guess costs an attacker little;
  a password hash, such as Argon2, is slow on purpose.
- **signing.** A digest proves a message unchanged only where the digest
  itself comes from someone trusted. A message and its digest sent
  together can both be changed; HMAC or a signature keeps them apart.
- **checking for accidents only.** CRC-32, in the
  [crc library](../crc/README.md), is far cheaper and catches every short
  burst of errors; use it where no one is trying to fool the check.

MD5, in the [md5 library](../md5/src/md5.fn), is broken: two messages
with one MD5 digest can be made in seconds. Where a format leaves the
choice open, take SHA-256.

## Hashing in pieces

`sha256(bytes)` takes a whole message. For one that arrives in pieces,
such as a file read a block at a time, `Sha256` holds a hash part way
through:

- `Sha256.empty` is the hash of no bytes yet.
- `hasher.update(bytes)` answers it with `bytes` taken in after what it
  has been given. It is a value like any other, so the one it was made
  from is unchanged, and two messages sharing a prefix can share the
  hasher that took it.
- `hasher.digest()` is the `Digest` of every byte given. Reading it
  changes nothing, so more may be given after.

Splitting a message anywhere gives the same digest as taking it whole. A
hasher keeps at most 63 bytes besides its state, so a message of any size
can be hashed a piece at a time without holding it all.

## A digest

`Digest` is what hashing answers, and only hashing makes one, so a
digest is never the wrong length nor mistaken for other bytes:

- `digest.to_bytes()` is its 32 bytes, each word of the state most
  significant byte first, as FIPS 180-4 writes them.
- `digest.to_hex()` is its 64 lowercase hex digits, as `sha256sum` prints
  them, and is what `"${digest}"` writes.
- Two digests are `==` where their bytes are, and a digest may key a
  `Map` or go in a `Set`.

To check a message against a digest published as hex, compare
`sha256(message).to_hex()` with it.

## Its names

The names follow the [crc library](../crc/README.md)'s, and Faun's
[naming](../../../../../notes/forms/naming.md): `empty` for the hash of
nothing, `update` to take bytes in, `sha256` for the one-shot function
and `to_bytes` and `to_hex` for conversions. Two differ from Rust's
[`sha2`](https://docs.rs/sha2), whose `Sha256::new`, `update` and
`finalize` this follows otherwise:

- **`digest`, not `finalize`.** A hasher is a value, so taking its
  digest finishes nothing, and more may follow; the name says what it
  answers, as the md5 library's `digest` does.
- **`digest`, not `value`.** The crc library's checksums answer a bare
  `U32`, its `value`. A SHA-256 answers a `Digest`, so the method is
  named for it.

## Speed

The library is written as FIPS 180-4 teaches the algorithm: each block
made into 16 words, grown to 64, and mixed by 64 rounds into eight words
of state. On x86-64 it hashes a megabyte in about 225 million
instructions, three and a half times the 67 million of the plain C twin
the [ratchet](../../../../../RATCHET.md) measures it against, in
[`eval/workloads/sha256`](../../../../../eval/workloads/sha256).

## Examples

[`examples/basic.fn`](examples/basic.fn) prints the digest of its
standard input, as `sha256sum` does:

```sh
$ printf abc | faun run examples/basic.fn
ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  -
```

[`examples/stream.fn`](examples/stream.fn) prints the same, reading its
input 64 KiB at a time and feeding each piece to a `Sha256`, so a file
of any size is hashed without holding it all.

## Using it

Name the library's root in your package's `uses`, by its path from your
package, as above. Published on its own, it would be named by the URL of
its bundle, which `faun bundle src/sha256.fn` makes.

## License

[Zero-Clause BSD](../../../../../LICENSE), as Faun is.
