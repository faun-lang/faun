# crc

Checksums over bytes for Faun: CRC-32, as zlib, gzip, PNG and ZIP compute
it, and Adler-32, as zlib's container ends with. Each is answered for a
whole message at once, or folded in a piece at a time.

```ruby ignore
package {
  uses: {
    crc: "../src/crc"
  }
}

import crc {Crc32, Adler32, crc32, adler32}

expect crc32("123456789".to_utf8()) == 0xcbf43926
expect adler32("Wikipedia".to_utf8()) == 0x11e60398

# The same checksum, folded in two pieces.
expect Crc32.empty.update("1234".to_utf8()).update("56789".to_utf8()).value() == 0xcbf43926

# Carried on from a checksum taken earlier.
expect Crc32.resume(crc32("1234".to_utf8())).update("56789".to_utf8()).value() == 0xcbf43926
```

Every expectation above is one of the library's own, in
[`src/crc.fn`](src/crc.fn), beneath the definition it shows.

## Which to use

CRC-32 is the one to reach for: it is what file formats and network
protocols check, and it catches every error burst of up to 32 bits.
Adler-32 is cheaper to compute but weak on short messages, whose sums
cover little of their range; use it where a format asks for it, as zlib's
does. Neither is a cryptographic hash. Each catches accidents, not
tampering: anyone can change a message and keep its checksum.

## Folding in pieces

`crc32(bytes)` and `adler32(bytes)` take a whole message. For one that
arrives in pieces, `Crc32` and `Adler32` hold a checksum part way through:

- `Crc32.empty` is the checksum of no bytes yet.
- `crc.update(bytes)` answers it with `bytes` folded in after what it has
  seen. It is a value like any other, so the one it was made from is
  unchanged.
- `crc.value()` is the checksum of every byte folded in, a `U32`. Reading
  it changes nothing, so more may be folded in after.
- `Crc32.resume(value)` carries on from a checksum taken earlier, such as
  one stored in a file's header, so only the bytes after it need reading.

`Adler32` has the same four. Splitting a message anywhere gives the same
checksum as taking it whole.

## Speed

CRC-32 reads eight bytes a step through eight tables of 256, the
slicing-by-8 algorithm zlib and Rust's `crc32fast` use when they have no
vector instructions. Adler-32 sums 64-bit words, reducing them only every
4 MB rather than every 5552 bytes as zlib's 32-bit sums must. On x86-64,
CRC-32 checks about 850 MB a second and Adler-32 about 1.1 GB, against
zlib's 8.6 and 5.2 GB, which reads its bytes several at a time and sums
in vector registers. The tables are built once a run, however many
checksums are taken.

## Example

[`examples/basic.fn`](examples/basic.fn) prints both checksums of its
standard input, in hex:

```sh
$ printf 123456789 | faun run examples/basic.fn
crc32 cbf43926
adler32 091e01de
```

## Using it

Name the library's root in your package's `uses`, by its path from your
package, as above. Published on its own, it would be named by the URL of
its bundle, which `faun bundle src/crc.fn` makes.

## License

[Zero-Clause BSD](../../../../../LICENSE), as Faun is.
