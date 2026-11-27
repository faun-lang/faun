# png

PNG images for Faun: decoding any PNG file into an `Image` of 8-bit
samples, and encoding an `Image` as a PNG file that decodes back to it
exactly. Built on the [flate](../flate) library's zlib streams and the
[crc](../crc) library's CRC-32.

```ruby ignore
package {
  uses: {
    png: "../src/png"
  }
}

import png {Image, decode, encode, encode_with, read_info, Level}

# An image is checked when it is made: its size, colour and pixels agree.
expect Image.from_pixels(2, 1, Rgb, [255, 0, 0, 0, 0, 255]).map_ok(Image.width) == Ok(2)
expect Image.from_pixels(2, 1, Rgb, [255, 0, 0]) == Err(WrongLength({expected: 6, found: 3}))

# Encoding cannot fail, and decoding gives the image back.
expect Image.from_pixels(2, 1, Rgb, [255, 0, 0, 0, 0, 255]).map_ok(|image| decode(encode(image)) == Ok(image)) == Ok(True)

# How hard deflate looks, and how each row is filtered, can be chosen.
expect Image.from_pixels(2, 1, Grey, [0, 255])
  .map_ok(|image| decode(encode_with(image, {level: Level.best, filter: Paeth})) == Ok(image)) == Ok(True)

# Fewer channels where they hold the image exactly: a smaller file.
expect Image.from_pixels(1, 1, Rgba, [1, 2, 3, 255]).map_ok(|image| image.reduced().color()) == Ok(Rgb)

# A file that is not PNG says why.
expect decode([]) == Err(NotPng)
```

Every expectation above is one of the library's own, in
[`src/png.fn`](src/png.fn), beneath the definition it shows.

## Images

An `Image` is a width, a height, a `Color` and its pixels, rows top to
bottom and each left to right, a byte a channel:

| `Color`     | Channels a pixel        |
| ----------- | ----------------------- |
| `Grey`      | grey                    |
| `GreyAlpha` | grey, alpha             |
| `Rgb`       | red, green, blue        |
| `Rgba`      | red, green, blue, alpha |

Alpha is opacity: 0 is transparent and 255 opaque. The type's constructor
is private, so an image is made only by `Image.from_pixels(width, height,
color, pixels)`, which answers `Err` naming what disagrees, `ZeroSize`,
`TooLarge` or `WrongLength({expected, found})`, or by `decode`. Every
image there is can be encoded, so `encode` answers the file itself, never
a `Result`.

Reading one: `image.width()`, `image.height()`, `image.color()`,
`image.pixels()`, and `image.pixel(x, y)`, the channels of one pixel or
`Err(OutOfBounds)`. Converting one: `image.to_rgba()` gives every pixel
four channels, and `image.reduced()` the fewest channels that hold the
image exactly. Images compare with `==` and print with `"${image}"`.

## Decoding

`decode(bytes)` reads every colour type at every bit depth PNG allows,
interlaced or not, and answers the image in the `Color` nearest what the
file stores:

| The file stores            | Depths         | Decodes to                    |
| -------------------------- | -------------- | ----------------------------- |
| grey                       | 1, 2, 4, 8, 16 | `Grey`, `GreyAlpha` with tRNS |
| grey and alpha             | 8, 16          | `GreyAlpha`                   |
| red, green and blue        | 8, 16          | `Rgb`, `Rgba` with tRNS       |
| red, green, blue and alpha | 8, 16          | `Rgba`                        |
| palette indices            | 1, 2, 4, 8     | `Rgb`, `Rgba` with tRNS       |

A 16-bit sample keeps its high byte, as libpng's `png_set_strip_16` does,
and a narrower one is scaled to the full range, so 1-bit white is 255.
`read_info(bytes)` reads only the header, as an `Info`: the size, the
`ColorType` and depth the file stores, and whether it is interlaced.

Corrupt or cut-short input answers a `PngError` naming why, such as
`NotPng`, `Truncated`, `ChecksumMismatch` or `BadImageData(BadCode)`;
nothing panics or reads past the end. Every chunk's CRC-32 is checked,
and so is the image data's Adler-32. The library decodes all of
[PngSuite](http://www.schaik.com/pngsuite/) to the pixels pypng gives and
refuses every file PngSuite marks corrupt.

## Encoding

`encode(image)` writes the image in its own colour, 8 bits a sample,
without interlacing, compressed at zlib's default level with each row
filtered as libpng chooses, by the smallest sum of its bytes taken as
signed. `encode_with(image, {level, filter})` chooses both: a `Level`
from `Level.none`, stored, through `Level.fastest` to `Level.best`, or
`Level.of_nat(n)` for zlib's 0 to 9; and a `Filter`, `Adaptive`,
`Unfiltered`, or one of PNG's `Sub`, `Up`, `Average` and `Paeth`
throughout. Files are within a percent of the size Pillow writes; call
`reduced()` first to drop channels the image does not need.

## What it leaves out

- Writing palettes, bit depths other than 8, or interlaced files.
- Reading 16-bit samples whole: decoding keeps their high byte.
- Ancillary chunks: gamma, colour spaces, ICC profiles, text, time and
  physical size are skipped, and nothing writes them. The samples come
  back as the file holds them, uncorrected.
- Animated PNG: an APNG decodes to its default image, its frames skipped.
- Streaming: a file is decoded and encoded whole, in memory.

## Speed

The library is written in the most natural Faun spelling: where that runs
slowly, the compiler has a bug to fix, and
[notes/compression.md](../../../../../notes/compression.md) lists them.
On x86-64 it decodes about 20 MB of pixels a second, nearly all of it
inflating, and encodes about 3 MB a second at the default level, against
Pillow's 380 and 17. The ratchet's suite measures a round trip through
it, [`eval/workloads/png`](../../../../../eval/workloads/png).

## Examples

[`examples/topam.fn`](examples/topam.fn) decodes a PNG file on standard
input and writes a PAM file of red, green, blue and alpha, which most
image tools read; [`examples/topng.fn`](examples/topng.fn) is its
reverse, taking `-0` to `-9` and `--unfiltered`:

```sh
$ faun run examples/topam.fn < fixtures/pngsuite/basn2c08.png > image.pam
$ faun run examples/topng.fn -- -9 < image.pam > image.png
```

## Using it

Name the library's root in your package's `uses`, by its path from your
package, as above. Published on its own, it would be named by the URL of
its bundle, which `faun bundle src/png.fn` makes.

## License

[Zero-Clause BSD](../../../../../LICENSE), as Faun is. PngSuite, in
[`fixtures/pngsuite`](fixtures/pngsuite), is Willem van Schaik's, under
[its own license](fixtures/pngsuite/PngSuite.LICENSE).
