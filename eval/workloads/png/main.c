/* Encode two images as PNG files and read them back: a square of `size`
   pixels a side in red, green, blue and alpha, gradients with a little
   noise, and one in grey. Each is encoded with its rows filtered as libpng
   chooses and compressed through zlib at its default level, then decoded,
   and printed with its size, the CRC-32 of the pixels read back and
   whether they are the pixels written. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <zlib.h>

static unsigned noise(unsigned x, unsigned y) { return (x * 7 + y * 13) * (x + 3 * y + 1) % 17; }

static void rgba(unsigned x, unsigned y, unsigned char *pixel) {
    pixel[0] = (unsigned char)(x * 4 + noise(x, y));
    pixel[1] = (unsigned char)(y * 4);
    pixel[2] = (unsigned char)(x + y + noise(x, y));
    pixel[3] = (unsigned char)(255 - (x ^ y) % 64);
}

static void grey(unsigned x, unsigned y, unsigned char *pixel) {
    pixel[0] = (unsigned char)(x * y / 4 + noise(x, y));
}

static const unsigned char signature[8] = {0x89, 'P', 'N', 'G', 0x0D, 0x0A, 0x1A, 0x0A};

static void put_word(unsigned char *at, unsigned long n) {
    at[0] = (unsigned char)(n >> 24);
    at[1] = (unsigned char)(n >> 16);
    at[2] = (unsigned char)(n >> 8);
    at[3] = (unsigned char)n;
}

static unsigned long word(const unsigned char *at) {
    return (unsigned long)at[0] << 24 | (unsigned long)at[1] << 16 | (unsigned long)at[2] << 8 | at[3];
}

/* A chunk written at `out`: its length, type, body and CRC-32. Answers
   the bytes written. */
static size_t put_chunk(unsigned char *out, const char *type, const unsigned char *body, size_t length) {
    put_word(out, length);
    memcpy(out + 4, type, 4);
    if (length) memcpy(out + 8, body, length);
    put_word(out + 8 + length, crc32(0, out + 4, (uInt)(length + 4)));
    return length + 12;
}

static unsigned char paeth(unsigned char a, unsigned char b, unsigned char c) {
    int p = a + b - c, pa = abs(p - a), pb = abs(p - b), pc = abs(p - c);
    if (pa <= pb && pa <= pc) return a;
    return pb <= pc ? b : c;
}

/* What filter `kind` predicts of the byte at `i` of `row`, `above` being
   the row before, and `per` the bytes a pixel. */
static unsigned char predicted(int kind, const unsigned char *row, const unsigned char *above, size_t i,
                               size_t per) {
    unsigned char a = i >= per ? row[i - per] : 0, b = above[i], c = i >= per ? above[i - per] : 0;
    switch (kind) {
    case 1: return a;
    case 2: return b;
    case 3: return (unsigned char)((a + b) / 2);
    case 4: return paeth(a, b, c);
    default: return 0;
    }
}

/* The PNG file of an image `width` by `height`, `per` bytes a pixel, of
   colour type `type`; its length is put in `length`. */
static unsigned char *encode(const unsigned char *pixels, unsigned width, unsigned height, size_t per,
                             int type, size_t *length) {
    size_t stride = width * per, raw_length = height * (stride + 1), at = 0, i;
    unsigned char *raw = malloc(raw_length), *zero = calloc(stride, 1), header[13];
    uLongf packed_length = compressBound(raw_length);
    unsigned char *packed = malloc(packed_length), *file;
    unsigned y;
    for (y = 0; y < height; y++) {
        const unsigned char *row = pixels + y * stride, *above = y ? row - stride : zero;
        unsigned char *out = raw + y * (stride + 1);
        unsigned long best_sum = (unsigned long)-1;
        int kind, best = 0;
        /* The filter whose residuals, taken as signed, sum smallest. */
        for (kind = 0; kind < 5; kind++) {
            unsigned long sum = 0;
            for (i = 0; i < stride; i++) {
                unsigned char r = (unsigned char)(row[i] - predicted(kind, row, above, i, per));
                sum += r < 128 ? r : 256 - r;
            }
            if (sum < best_sum) {
                best_sum = sum;
                best = kind;
            }
        }
        out[0] = (unsigned char)best;
        for (i = 0; i < stride; i++) out[i + 1] = (unsigned char)(row[i] - predicted(best, row, above, i, per));
    }
    compress2(packed, &packed_length, raw, raw_length, Z_DEFAULT_COMPRESSION);
    file = malloc(8 + 25 + packed_length + (packed_length / 65536 + 1) * 12 + 12);
    memcpy(file, signature, 8);
    at = 8;
    put_word(header, width);
    put_word(header + 4, height);
    header[8] = 8;
    header[9] = (unsigned char)type;
    header[10] = header[11] = header[12] = 0;
    at += put_chunk(file + at, "IHDR", header, 13);
    for (i = 0; i < packed_length; i += 65536) {
        size_t piece = packed_length - i < 65536 ? packed_length - i : 65536;
        at += put_chunk(file + at, "IDAT", packed + i, piece);
    }
    at += put_chunk(file + at, "IEND", NULL, 0);
    free(raw);
    free(zero);
    free(packed);
    *length = at;
    return file;
}

/* The pixels of a PNG file of 8-bit samples without interlacing, as the
   encoder writes them, every chunk's CRC-32 checked; or NULL. Its size
   and bytes a pixel are put in `width`, `height` and `per`. */
static unsigned char *decode(const unsigned char *file, size_t length, unsigned *width, unsigned *height,
                             size_t *per) {
    size_t at = 8, data_length = 0, stride, i;
    unsigned char *data = malloc(length), *raw, *pixels;
    uLongf raw_length;
    unsigned y;
    int seen_header = 0;
    if (length < 8 || memcmp(file, signature, 8) != 0) return NULL;
    for (;;) {
        unsigned long size;
        const unsigned char *body;
        if (at + 12 > length) return NULL;
        size = word(file + at);
        body = file + at + 8;
        if (at + 12 + size > length) return NULL;
        if (crc32(0, file + at + 4, (uInt)(size + 4)) != word(body + size)) return NULL;
        if (memcmp(file + at + 4, "IHDR", 4) == 0) {
            static const size_t channels[7] = {1, 0, 3, 0, 2, 0, 4};
            if (size != 13 || body[8] != 8 || body[9] > 6 || !channels[body[9]] || body[12] != 0) return NULL;
            *width = (unsigned)word(body);
            *height = (unsigned)word(body + 4);
            *per = channels[body[9]];
            seen_header = 1;
        } else if (memcmp(file + at + 4, "IDAT", 4) == 0) {
            memcpy(data + data_length, body, size);
            data_length += size;
        } else if (memcmp(file + at + 4, "IEND", 4) == 0) {
            break;
        }
        at += 12 + size;
    }
    if (!seen_header) return NULL;
    stride = *width * *per;
    raw_length = *height * (stride + 1);
    raw = malloc(raw_length);
    pixels = malloc(*height * stride);
    if (uncompress(raw, &raw_length, data, data_length) != Z_OK || raw_length != *height * (stride + 1))
        return NULL;
    for (y = 0; y < *height; y++) {
        const unsigned char *in = raw + y * (stride + 1);
        unsigned char *row = pixels + y * stride;
        const unsigned char *above = y ? row - stride : NULL;
        if (in[0] > 4) return NULL;
        for (i = 0; i < stride; i++) {
            unsigned char a = i >= *per ? row[i - *per] : 0, b = above ? above[i] : 0;
            unsigned char c = above && i >= *per ? above[i - *per] : 0;
            switch (in[0]) {
            case 0: row[i] = in[i + 1]; break;
            case 1: row[i] = (unsigned char)(in[i + 1] + a); break;
            case 2: row[i] = (unsigned char)(in[i + 1] + b); break;
            case 3: row[i] = (unsigned char)(in[i + 1] + (a + b) / 2); break;
            default: row[i] = (unsigned char)(in[i + 1] + paeth(a, b, c)); break;
            }
        }
    }
    free(data);
    free(raw);
    return pixels;
}

/* The image `size` pixels square, `per` bytes a pixel of colour type
   `type`, each made by `pixel`, encoded and decoded, and reported. */
static void square(const char *name, unsigned size, size_t per, int type,
                   void (*pixel)(unsigned, unsigned, unsigned char *)) {
    unsigned char *pixels = malloc((size_t)size * size * per), *file, *back;
    unsigned x, y, width, height;
    size_t length, back_per;
    for (y = 0; y < size; y++)
        for (x = 0; x < size; x++) pixel(x, y, pixels + ((size_t)y * size + x) * per);
    file = encode(pixels, size, size, per, type, &length);
    back = decode(file, length, &width, &height, &back_per);
    if (!back) {
        printf("%s error\n", name);
    } else {
        size_t bytes = (size_t)width * height * back_per;
        int same = width == size && height == size && back_per == per && memcmp(back, pixels, bytes) == 0;
        printf("%s %ux%u crc32 %08lx %s\n", name, width, height, crc32(0, back, (uInt)bytes),
               same ? "matches" : "differs");
    }
    free(pixels);
    free(file);
    free(back);
}

int main(int argc, char **argv) {
    unsigned size = argc > 1 ? (unsigned)atol(argv[1]) : 64;
    square("rgba", size, 4, 6, rgba);
    square("grey", size, 1, 0, grey);
    return 0;
}
