/* Mutation fuzzer for ipdb_open/ipap_parse. Build with ASan+UBSan (make fuzz).
 * Most mutations re-seal the CRC so they reach the structural checks; whenever the reader
 * accepts a mutated file, every accessor path is walked to prove acceptance means bounds-safe.
 *   ipdb_fuzz library.ipdb [iterations] [seed]
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "ipdb.h"

static uint32_t rng_state;
static uint32_t rnd(void)
{
    rng_state ^= rng_state << 13;
    rng_state ^= rng_state >> 17;
    rng_state ^= rng_state << 5;
    return rng_state;
}

static volatile uint32_t sink;

static void touch_str(const ipdb_db *db, uint32_t off)
{
    sink += (uint32_t)strlen(ipdb_str(db, off));
}

static void walk(const ipdb_db *db)
{
    for (uint32_t i = 0; i < db->n_tracks; i++) {
        const ipdb_track *t = ipdb_track_at(db, i);
        touch_str(db, t->path);
        touch_str(db, t->title);
        touch_str(db, db->artists[t->artist_id].name);
        touch_str(db, db->albums[t->album_id].title);
        if (t->genre_id != IPDB_NONE)
            touch_str(db, db->genres[t->genre_id].name);
        if (t->composer_id != IPDB_NONE)
            touch_str(db, db->composers[t->composer_id].name);
    }
    for (uint32_t a = 0; a < db->n_albums; a++) {
        const ipdb_album *al = ipdb_album_at(db, a);
        touch_str(db, al->title);
        for (uint32_t k = 0; k < al->tracks_count; k++)
            touch_str(db, ipdb_track_at(db, ipdb_album_track(db, al, k))->title);
    }
    for (uint32_t r = 0; r < db->n_artists; r++)
        for (uint32_t k = 0; k < db->artists[r].count; k++)
            touch_str(db, db->albums[db->artist_albums[db->artists[r].first + k]].title);
    for (uint32_t g = 0; g < db->n_genres; g++)
        for (uint32_t k = 0; k < db->genres[g].count; k++)
            touch_str(db, db->artists[db->genre_artists[db->genres[g].first + k]].name);
    for (uint32_t c = 0; c < db->n_composers; c++)
        for (uint32_t k = 0; k < db->composers[c].count; k++)
            touch_str(db, db->tracks[db->composer_tracks[db->composers[c].first + k]].title);
    for (uint32_t p = 0; p < db->n_playlists; p++)
        for (uint32_t k = 0; k < db->playlists[p].count; k++)
            touch_str(db, db->tracks[db->playlist_tracks[db->playlists[p].first + k]].title);
    for (int r = 0; r < IPDB_JUMP_ROWS; r++)
        for (int b = 0; b < IPDB_JUMP_BUCKETS; b++)
            sink += ipdb_jump(db, r, b);
}

static void reseal(uint8_t *b, size_t len)
{
    uint32_t fsize;
    if (len < 64)
        return;
    memcpy(&fsize, b + 20, 4);
    if (fsize < 64 || fsize > len)
        return;
    uint32_t crc = ipdb_crc32(0, b + 64, fsize - 64);
    memcpy(b + 40, &crc, 4);
}

static const uint32_t interesting[] = { 0, 1, 2, 15, 16, 63, 64, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFE, 0xFFFFFFFF };

int main(int argc, char **argv)
{
    if (argc < 2) {
        fprintf(stderr, "usage: %s library.ipdb [iterations] [seed]\n", argv[0]);
        return 2;
    }
    long iters = argc > 2 ? atol(argv[2]) : 20000;
    rng_state = argc > 3 ? (uint32_t)strtoul(argv[3], NULL, 10) : 0x9E3779B9u;
    if (!rng_state)
        rng_state = 1;

    FILE *f = fopen(argv[1], "rb");
    if (!f) {
        perror(argv[1]);
        return 1;
    }
    fseek(f, 0, SEEK_END);
    size_t len = (size_t)ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *orig = malloc(len);
    if (!orig || fread(orig, 1, len, f) != len) {
        fprintf(stderr, "read failed\n");
        return 1;
    }
    fclose(f);

    ipdb_db db;
    if (ipdb_open(&db, orig, len) != IPDB_OK) {
        fprintf(stderr, "input does not validate\n");
        return 1;
    }
    walk(&db);

    long accepted = 0, rejected[16] = { 0 };
    for (long it = 0; it < iters; it++) {
        size_t n = len;
        int mode = (int)(rnd() % 6);
        if (mode == 5)
            n = rnd() % (len + 1); /* truncation */
        /* Exact-size allocation so ASan catches any overread. */
        uint8_t *b = malloc(n ? n : 1);
        memcpy(b, orig, n);
        int muts = 1 + (int)(rnd() % 4);
        for (int m = 0; m < muts && n > 4; m++) {
            size_t pos = rnd() % (n - 3);
            if (mode == 0 || mode == 5) {
                b[pos] ^= (uint8_t)(1u << (rnd() % 8));
            } else if (mode == 1 || mode == 2) {
                uint32_t v = interesting[rnd() % (sizeof interesting / sizeof interesting[0])];
                pos &= ~(size_t)3;
                if (pos + 4 <= n)
                    memcpy(b + pos, &v, 4);
            } else if (mode == 3) {
                b[pos] = (uint8_t)rnd();
            } else if (n >= 64) {
                /* header and section table fields */
                uint32_t v = (rnd() & 1) ? interesting[rnd() % 11] : rnd() % (uint32_t)(len + 64);
                size_t hp = 8 + 4 * (rnd() % 60);
                if (hp + 4 <= n)
                    memcpy(b + hp, &v, 4);
            }
        }
        if (mode != 3)
            reseal(b, n);
        int err = ipdb_open(&db, b, n);
        if (err == IPDB_OK) {
            accepted++;
            walk(&db);
        } else if (err > 0 && err < 16) {
            rejected[err]++;
        }
        free(b);
    }

    /* Art pack header fuzzing against a synthetic valid header. */
    uint8_t head[64 + 16];
    memset(head, 0, sizeof head);
    memcpy(head, "IPAP", 4);
    head[4] = 1;
    head[8] = 64;
    head[12] = 1;
    uint32_t art_count = 3, data_off = 4096;
    uint64_t fsz = 4096 + 3 * 28 * 28 * 2, coff = 4096;
    memcpy(head + 16, &art_count, 4);
    memcpy(head + 20, &data_off, 4);
    memcpy(head + 32, &fsz, 8);
    memcpy(head + 64, "THMB", 4);
    head[68] = 28;
    head[70] = 28;
    memcpy(head + 72, &coff, 8);
    long art_ok = 0;
    for (long it = 0; it < iters; it++) {
        uint8_t h[sizeof head];
        memcpy(h, head, sizeof h);
        if (it) {
            size_t pos = (rnd() % (sizeof h / 4)) * 4;
            uint32_t v = (rnd() & 1) ? interesting[rnd() % 11] : rnd();
            memcpy(h + pos, &v, 4);
        }
        memset(h + 40, 0, 4);
        uint32_t crc = ipdb_crc32(0, h, sizeof h);
        memcpy(h + 40, &crc, 4);
        ipap_pack p;
        if (ipap_parse(&p, h, sizeof h, fsz) == IPDB_OK) {
            art_ok++;
            uint64_t off;
            uint32_t sz;
            uint16_t w, hh;
            for (uint32_t a = 0; a < p.art_count && a < 8; a++)
                if (!ipap_slot(&p, IPAP_THMB, a, &off, &sz, &w, &hh) && off + sz > p.file_size) {
                    fprintf(stderr, "art slot out of file bounds accepted\n");
                    return 1;
                }
        }
    }

    printf("ipdb: %ld iterations, %ld accepted;", iters, accepted);
    for (int e = 1; e < 16; e++)
        if (rejected[e])
            printf(" %s=%ld", ipdb_strerror(e), rejected[e]);
    printf("\nipap: %ld iterations, %ld accepted\n", iters, art_ok);
    free(orig);
    return 0;
}
