/* Cross-check the device font reader against atlases built by `ipdb fonts`.
 * Renders a sample string to a PGM so the glyphs can be compared with the host preview.
 *   ipfn_test FONT.ipfn [out.pgm]
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "ipfn.h"

#define W 640
#define H 64

static unsigned char canvas[H][W];

int main(int argc, char **argv)
{
    if (argc < 2) {
        fprintf(stderr, "usage: %s FONT.ipfn [out.pgm]\n", argv[0]);
        return 2;
    }
    FILE *fp = fopen(argv[1], "rb");
    if (!fp) {
        perror(argv[1]);
        return 1;
    }
    fseek(fp, 0, SEEK_END);
    long len = ftell(fp);
    fseek(fp, 0, SEEK_SET);
    void *buf = NULL;
    if (posix_memalign(&buf, sizeof(void *), (size_t)len) != 0 || fread(buf, 1, (size_t)len, fp) != (size_t)len) {
        fprintf(stderr, "read failed\n");
        return 1;
    }
    fclose(fp);

    ipfn_font f;
    int err = ipfn_open(&f, buf, (size_t)len);
    if (err) {
        fprintf(stderr, "%s: %s\n", argv[1], ipfn_strerror(err));
        return 1;
    }
    printf("%s: %upx ascent %u descent %u line %u, %u glyphs, %u ranges, tracking %d\n",
           argv[1], f.px, f.ascent, f.descent, f.line_height, f.n_glyphs, f.n_ranges, f.tracking);

    const char *sample = "Halcyon Drift \xe2\x80\xa2 Low Tide Lights 3:00 \xc3\x85\xc3\x98\xe2\x80\x94";
    int w = ipfn_width(&f, sample);
    printf("  width(sample) = %d px\n", w);

    /* UTF-8 decoding edge cases */
    const char *p = "\xff\xfe";
    if (ipfn_utf8_next(&p) != 0xFFFD) {
        fprintf(stderr, "invalid utf-8 not replaced\n");
        return 1;
    }
    p = "\xc0\xaf"; /* overlong '/' */
    if (ipfn_utf8_next(&p) != 0xFFFD) {
        fprintf(stderr, "overlong form accepted\n");
        return 1;
    }
    p = "\xed\xa0\x80"; /* surrogate */
    if (ipfn_utf8_next(&p) != 0xFFFD) {
        fprintf(stderr, "surrogate accepted\n");
        return 1;
    }
    p = "\xe2\x80"; /* truncated */
    ipfn_utf8_next(&p);

    /* ipfn_fit must never exceed the limit and must land on a character boundary */
    for (int limit = 0; limit <= w + 10; limit += 7) {
        size_t n = ipfn_fit(&f, sample, limit);
        if (n > strlen(sample)) {
            fprintf(stderr, "fit past end\n");
            return 1;
        }
        if (n && (sample[n] & 0xC0) == 0x80) {
            fprintf(stderr, "fit split a utf-8 sequence at %d\n", limit);
            return 1;
        }
        if (ipfn_width_n(&f, sample, n) > limit) {
            fprintf(stderr, "fit overflows limit %d\n", limit);
            return 1;
        }
    }
    printf("  utf-8 and fit checks passed\n");

    /* draw */
    memset(canvas, 0, sizeof canvas);
    int pen = 4, baseline = f.ascent + 4;
    for (const char *s = sample; *s;) {
        ipfn_glyph g;
        ipfn_get_glyph(&f, ipfn_utf8_next(&s), &g);
        for (int row = 0; row < g.h; row++)
            for (int col = 0; col < g.w; col++) {
                int x = pen + g.left + col, y = baseline - g.top + row;
                if (x >= 0 && x < W && y >= 0 && y < H) {
                    unsigned v = canvas[y][x] + g.bitmap[row * g.w + col];
                    canvas[y][x] = v > 255 ? 255 : (unsigned char)v;
                }
            }
        pen += g.advance + f.tracking;
    }
    if (pen - f.tracking - 4 != w) {
        fprintf(stderr, "pen advance %d disagrees with ipfn_width %d\n", pen - f.tracking - 4, w);
        free(buf);
        return 1;
    }

    if (argc > 2) {
        FILE *o = fopen(argv[2], "wb");
        int height = f.line_height + 8;
        if (height > H)
            height = H;
        fprintf(o, "P5\n%d %d\n255\n", w + 8, height);
        for (int y = 0; y < height; y++)
            fwrite(canvas[y], 1, (size_t)w + 8, o);
        fclose(o);
        printf("  wrote %s\n", argv[2]);
    }
    free(buf);
    return 0;
}
