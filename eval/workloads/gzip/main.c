/* Gzip standard input at the default level, gunzip what that wrote, and
   check it is the input again, through zlib: deflate's search and codes,
   CRC-32, and inflate. Prints the length read back, its CRC-32 and whether
   the round trip matched. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <zlib.h>

/* zlib's window bits, plus 16 for a gzip header and trailer rather than
   zlib's. */
#define GZIP_WINDOW (15 + 16)

/* All of standard input, its length in *length. */
static unsigned char *read_all(size_t *length) {
    size_t size = 1 << 16, used = 0, got;
    unsigned char *data = malloc(size);
    while ((got = fread(data + used, 1, size - used, stdin)) > 0) {
        used += got;
        if (used == size) {
            size *= 2;
            data = realloc(data, size);
        }
    }
    *length = used;
    return data;
}

/* data as a gzip file of one member, its length in *length. */
static unsigned char *gzip(const unsigned char *data, size_t size, size_t *length) {
    z_stream stream = {0};
    deflateInit2(&stream, Z_DEFAULT_COMPRESSION, Z_DEFLATED, GZIP_WINDOW, 8,
                 Z_DEFAULT_STRATEGY);
    size_t bound = deflateBound(&stream, size);
    unsigned char *out = malloc(bound);
    stream.next_in = (unsigned char *)data;
    stream.avail_in = size;
    stream.next_out = out;
    stream.avail_out = bound;
    deflate(&stream, Z_FINISH);
    *length = stream.total_out;
    deflateEnd(&stream);
    return out;
}

/* What a gzip file holds, every member of it, its length in *length; NULL
   where it is refused. */
static unsigned char *gunzip(const unsigned char *file, size_t size, size_t *length) {
    z_stream stream = {0};
    inflateInit2(&stream, GZIP_WINDOW);
    size_t capacity = 1 << 16, used = 0;
    unsigned char *out = malloc(capacity);
    stream.next_in = (unsigned char *)file;
    stream.avail_in = size;
    for (;;) {
        if (used == capacity) {
            capacity *= 2;
            out = realloc(out, capacity);
        }
        stream.next_out = out + used;
        stream.avail_out = capacity - used;
        int status = inflate(&stream, Z_NO_FLUSH);
        used = capacity - stream.avail_out;
        if (status == Z_STREAM_END && stream.avail_in == 0)
            break;
        if (status == Z_STREAM_END)
            inflateReset(&stream);
        else if (status != Z_OK) {
            inflateEnd(&stream);
            free(out);
            return NULL;
        }
    }
    inflateEnd(&stream);
    *length = used;
    return out;
}

int main(void) {
    size_t size, compressed, length;
    unsigned char *input = read_all(&size);
    unsigned char *file = gzip(input, size, &compressed);
    unsigned char *back = gunzip(file, compressed, &length);
    if (back == NULL) {
        printf("round trip failed\n");
        return 0;
    }
    int matched = length == size && memcmp(back, input, size) == 0;
    printf("length %zu\ncrc32 %lu\nround trip %s\n", length, crc32(0, back, length),
           matched ? "matched" : "differed");
    free(back);
    free(file);
    free(input);
    return 0;
}
