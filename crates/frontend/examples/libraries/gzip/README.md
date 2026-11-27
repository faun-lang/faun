# gzip

The gzip file format (RFC 1952) for Faun: bytes compressed into a gzip
file, with the name, time and comment a file carries in its header, and a
gzip file read back, every member of it, as `gunzip` reads one. What it
writes `gzip -d` reads, and what `gzip` writes it reads.

```ruby ignore
package {
  uses: {
    gzip: "../src/gzip"
  }
}

import gzip {compress, decompress, members, Header, Level}

expect decompress(compress("hello".to_utf8(), Level.default)) == Ok("hello".to_utf8())
expect decompress("hello".to_utf8()) == Err(NotGzip)

# Two members read as one file, as `cat a.gz b.gz` makes.
expect decompress(compress("ab".to_utf8(), Level.best).concat(compress("cd".to_utf8(), Level.none))) == Ok("abcd".to_utf8())

# A header with fields, read back with the member's data.
expect (
  named = Header.empty.with_name("a.txt".to_utf8()).with_mtime(1700000000)
  read = members(named.compress("one".to_utf8(), Level.best).concat(compress("two".to_utf8(), Level.fastest)))
  read == Ok([{header: named, data: "one".to_utf8()}, {header: Header.empty, data: "two".to_utf8()}])
)
```

Every expectation above is one of the library's own, in
[`src/gzip.fn`](src/gzip.fn), beneath the definition it shows.

## Writing

- `compress(data, level)` is `data` as a gzip file of one member, its
  header empty.
- `header.compress(data, level)` is the same behind `header`.
- `Level` says how hard to look for matches: `Level.none` stores the data
  as it is, `Level.fastest`, `Level.default` and `Level.best` are zlib's 1,
  6 and 9, and `Level.of_nat(n)` and `Level.from_nat(n)` take a number, the
  first making one above 9 into 9, the second refusing it. It is the flate
  library's own.

A `Header` is built from `Header.empty`, which has no name, no comment, no
extra field, no time and an unknown system, by methods that each answer a
header with one more field:

- `with_name(bytes)`: the original file's name.
- `with_comment(bytes)`: a comment for people to read.
- `with_extra(bytes)`: the extra field, subfields laid out as RFC 1952
  says, up to the 65535 bytes its length can say.
- `with_mtime(seconds)`: when the original file was last changed, a `U32`
  of seconds since the start of 1970 UTC; nought gives no time.
- `with_os(os)`: the system it was written on, an `Os`: `Unix`, `Ntfs`,
  `Fat`, `Macintosh` and the rest RFC 1952 numbers, or `Unknown`.

A name and a comment are bytes, which RFC 1952 says are ISO 8859-1 and
which `gzip` writes as the file system gives them. Each ends at a zero
byte in the file, so `with_name` and `with_comment` keep only what comes
before one: no header the library builds can be read back otherwise.

## Reading

- `decompress(file)` is every member's data, one after another, as
  `gunzip` writes it.
- `members(file)` is each member, in order, as a `Member`,
  `{header, data}`, so the header it read is there to see.

A header's fields are read by methods named for them, `name()`,
`comment()`, `extra()`, `mtime()` and `os()`; those a header may lack
answer `Err(Missing)` where it has none. Zero bytes after the last member
are padding, which `gzip` ignores and so does the library.

Reading checks everything the format lets it, and answers a `GzipError`
saying what it refused:

| Error                    | Why                                       |
| ------------------------ | ----------------------------------------- |
| `NotGzip`                | it does not begin with gzip's magic bytes |
| `UnsupportedMethod(m)`   | its method is `m`, not DEFLATE, 8         |
| `ReservedFlags`          | a flag RFC 1952 reserves is set           |
| `Truncated`              | it ends before the member does            |
| `HeaderChecksumMismatch` | the header's own CRC-16 is not its bytes' |
| `BadDeflate(why)`        | the compressed data is not DEFLATE: `why` |
| `ChecksumMismatch`       | the trailer's CRC-32 is not the data's    |
| `LengthMismatch`         | the trailer gives another length          |
| `TrailingData`           | bytes after the last member are not one   |

`why` is the flate library's `InflateError`, such as `BadCode` or
`BadDistance`.

## Speed

The work is the flate library's, deflate and inflate, and the crc
library's CRC-32: see theirs. On x86-64 a megabyte of text compresses at
the default level in about 0.3 seconds and reads back in 25 ms; a megabyte
of random bytes, which stores, in 80 ms and 5 ms. The whole file is held
in memory, compressed and read at once.

## Examples

[`examples/gzip.fn`](examples/gzip.fn) compresses standard input,
[`examples/gunzip.fn`](examples/gunzip.fn) reads it back, and with
`--list` says what each member's header holds: its length, time, system,
name and comment.

```sh
$ printf hello | faun run examples/gzip.fn -- --name hello.txt | gzip -dc
hello
$ printf hello | gzip -c | faun run examples/gunzip.fn
hello
$ printf hello | faun run examples/gzip.fn -- --name hello.txt | faun run examples/gunzip.fn -- --list
5 0 Unknown hello.txt -
```

## Using it

Name the library's root in your package's `uses`, by its path from your
package, as above. Published on its own, it would be named by the URL of
its bundle, which `faun bundle src/gzip.fn` makes.

## License

[Zero-Clause BSD](../../../../../LICENSE), as Faun is.
