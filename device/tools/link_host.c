/* Host harness: runs the device-side link protocol over stdin/stdout so the companion's
 * JavaScript client can be tested without hardware. Input is fed in random-sized pieces
 * to exercise frame reassembly.
 *   link_host [seed]
 */
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

#include "link_proto.h"

static struct link l;

static int out(void *ctx, const void *buf, size_t n)
{
    (void)ctx;
    const unsigned char *p = buf;
    while (n) {
        ssize_t w = write(1, p, n);
        if (w <= 0)
            return -1;
        p += w;
        n -= (size_t)w;
    }
    return 0;
}

int main(int argc, char **argv)
{
    srand(argc > 1 ? (unsigned)atoi(argv[1]) : 1);
    struct link_io io = { .write = out, .ctx = NULL, .hello = "host-harness" };
    link_init(&l, &io);
    static unsigned char buf[65536];
    ssize_t n;
    while ((n = read(0, buf, sizeof buf)) > 0) {
        size_t off = 0;
        while (off < (size_t)n) {
            size_t piece = 1 + (size_t)rand() % 5000;
            if (piece > (size_t)n - off)
                piece = (size_t)n - off;
            link_feed(&l, buf + off, piece);
            off += piece;
        }
    }
    fprintf(stderr, "link_host: rx %llu tx %llu frames %u crc_errors %u resyncs %u errors %u\n",
            (unsigned long long)l.st.rx_bytes, (unsigned long long)l.st.tx_bytes, l.st.frames,
            l.st.crc_errors, l.st.resyncs, l.st.errors);
    return 0;
}
