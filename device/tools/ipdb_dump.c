/* Host tool: load library.ipdb + artwork.ipap through the device reader and print them.
 *   ipdb_dump DIR [-a] [-t] [-x CLASS ART_ID OUT.ppm]
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "ipdb.h"

static void *slurp(const char *path, size_t *len)
{
    FILE *f = fopen(path, "rb");
    if (!f)
        return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    void *buf = malloc(n > 0 ? (size_t)n : 1);
    if (buf && n > 0 && fread(buf, 1, (size_t)n, f) != (size_t)n) {
        free(buf);
        buf = NULL;
    }
    fclose(f);
    *len = (size_t)n;
    return buf;
}

static void fmt_time(uint32_t ms, char *out, size_t n)
{
    snprintf(out, n, "%u:%02u", ms / 60000, (ms / 1000) % 60);
}

int main(int argc, char **argv)
{
    if (argc < 2) {
        fprintf(stderr, "usage: %s DIR [-a] [-t] [-x CLASS ART_ID OUT.ppm]\n", argv[0]);
        return 2;
    }
    int show_albums = 0, show_tracks = 0;
    const char *xclass = NULL, *xout = NULL;
    unsigned long xid = 0;
    for (int i = 2; i < argc; i++) {
        if (!strcmp(argv[i], "-a"))
            show_albums = 1;
        else if (!strcmp(argv[i], "-t"))
            show_tracks = 1;
        else if (!strcmp(argv[i], "-x") && i + 3 < argc) {
            xclass = argv[i + 1];
            xid = strtoul(argv[i + 2], NULL, 10);
            xout = argv[i + 3];
            i += 3;
        }
    }

    char path[4096];
    size_t len;
    snprintf(path, sizeof path, "%s/library.ipdb", argv[1]);
    void *buf = slurp(path, &len);
    if (!buf) {
        perror(path);
        return 1;
    }
    ipdb_db db;
    int err = ipdb_open(&db, buf, len);
    if (err) {
        fprintf(stderr, "%s: %s\n", path, ipdb_strerror(err));
        return 1;
    }
    printf("library: generation %llu, %u tracks, %u albums, %u artists, %u genres, %u composers, %u playlists\n",
           (unsigned long long)db.generation, db.n_tracks, db.n_albums, db.n_artists, db.n_genres, db.n_composers,
           db.n_playlists);

    snprintf(path, sizeof path, "%s/artwork.ipap", argv[1]);
    FILE *af = fopen(path, "rb");
    ipap_pack pack;
    int have_pack = 0;
    if (af) {
        uint8_t head[IPAP_HEAD_MAX];
        size_t got = fread(head, 1, sizeof head, af);
        fseek(af, 0, SEEK_END);
        uint64_t flen = (uint64_t)ftell(af);
        err = ipap_parse(&pack, head, got, flen);
        if (err)
            fprintf(stderr, "%s: %s\n", path, ipdb_strerror(err));
        else if (pack.generation != db.generation)
            fprintf(stderr, "%s: generation mismatch, ignoring\n", path);
        else
            have_pack = 1;
        if (have_pack)
            printf("artwork: %u covers, %u classes\n", pack.art_count, pack.class_count);
    }

    for (uint32_t a = 0; show_albums && a < db.n_albums; a++) {
        const ipdb_album *al = ipdb_album_at(&db, a);
        printf("[%u] %s - %s (%u)%s\n", a, ipdb_str(&db, al->title), ipdb_str(&db, db.artists[al->artist_id].name),
               al->year, (al->flags & IPDB_AF_COMPILATION) ? " [comp]" : "");
        for (uint32_t k = 0; k < al->tracks_count; k++) {
            const ipdb_track *t = ipdb_track_at(&db, ipdb_album_track(&db, al, k));
            char tm[16];
            fmt_time(t->duration_ms, tm, sizeof tm);
            printf("    %u-%02u %s  %s\n", t->disc_no, t->track_no, ipdb_str(&db, t->title), tm);
        }
    }
    for (uint32_t i = 0; show_tracks && i < db.n_tracks; i++) {
        const ipdb_track *t = ipdb_track_at(&db, i);
        printf("[%u] %s - %s  %s\n", i, ipdb_str(&db, t->title), ipdb_str(&db, db.artists[t->artist_id].name),
               ipdb_str(&db, t->path));
    }
    for (uint32_t p = 0; p < db.n_playlists; p++)
        printf("playlist %s (%u tracks)\n", ipdb_str(&db, db.playlists[p].name), db.playlists[p].count);

    int rc = 0;
    if (xclass) {
        uint64_t off;
        uint32_t size;
        uint16_t w, h;
        if (!have_pack || strlen(xclass) != 4 ||
            ipap_slot(&pack, IPAP_FOURCC(xclass[0], xclass[1], xclass[2], xclass[3]), (uint32_t)xid, &off, &size,
                      &w, &h)) {
            fprintf(stderr, "no such art slot\n");
            rc = 1;
        } else {
            uint8_t *px = malloc(size);
            FILE *o = fopen(xout, "wb");
            fseek(af, (long)off, SEEK_SET);
            if (!px || !o || fread(px, 1, size, af) != size) {
                fprintf(stderr, "art export failed\n");
                rc = 1;
            } else {
                fprintf(o, "P6\n%u %u\n255\n", w, h);
                for (uint32_t i = 0; i < (uint32_t)w * h; i++) {
                    uint16_t v = (uint16_t)(px[i * 2] | (px[i * 2 + 1] << 8));
                    uint8_t rgb[3] = { (uint8_t)(((v >> 11) & 31) * 255 / 31), (uint8_t)(((v >> 5) & 63) * 255 / 63),
                                       (uint8_t)((v & 31) * 255 / 31) };
                    fwrite(rgb, 1, 3, o);
                }
                printf("wrote %ux%u %s\n", w, h, xout);
            }
            if (o)
                fclose(o);
            free(px);
        }
    }
    if (af)
        fclose(af);
    free(buf);
    return rc;
}
