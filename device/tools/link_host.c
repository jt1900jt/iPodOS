/* Host harness: runs the device-side link protocol over stdin/stdout so the companion's
 * JavaScript client can be tested without hardware. Input is fed in random-sized pieces
 * to exercise frame reassembly.
 *   link_host [seed]
 */
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

#include <dirent.h>
#include <errno.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/statvfs.h>

#include "link_proto.h"

/* Host-side filesystem for the protocol tests. Everything is confined to a sandbox
 * directory given as argv[2], so a stray path cannot touch the rest of the machine. */
static char g_root[1024];
static FILE *g_put;
static char g_put_target[2048], g_put_tmp[2100];

static int real_path(char *out, size_t n, const char *path)
{
    if (strstr(path, ".."))
        return -1;
    return snprintf(out, n, "%s%s", g_root, path) < (int)n ? 0 : -1;
}

static int h_stat(void *c, const char *path, int *kind, uint32_t *size, uint32_t *mtime)
{
    (void)c;
    char rp[2048];
    struct stat st;
    *kind = 0; *size = 0; *mtime = 0;
    if (real_path(rp, sizeof rp, path) < 0)
        return -1;
    if (stat(rp, &st) != 0)
        return 0;
    *kind = S_ISDIR(st.st_mode) ? 2 : 1;
    *size = (uint32_t)st.st_size;
    *mtime = (uint32_t)st.st_mtime;
    return 0;
}

static int h_list(void *c, const char *path,
                  void (*emit)(void *, const char *, int, uint32_t, uint32_t), void *ectx)
{
    (void)c;
    char rp[2048];
    if (real_path(rp, sizeof rp, path) < 0)
        return -1;
    DIR *d = opendir(rp);
    if (!d)
        return -1;
    struct dirent *e;
    while ((e = readdir(d))) {
        if (!strcmp(e->d_name, ".") || !strcmp(e->d_name, ".."))
            continue;
        char child[3200];
        struct stat st;
        snprintf(child, sizeof child, "%s/%s", rp, e->d_name);
        if (stat(child, &st) != 0)
            continue;
        emit(ectx, e->d_name, S_ISDIR(st.st_mode) ? 2 : 1, (uint32_t)st.st_size, (uint32_t)st.st_mtime);
    }
    closedir(d);
    return 0;
}

static int h_mkdir(void *c, const char *path)
{
    (void)c;
    char rp[2048];
    if (real_path(rp, sizeof rp, path) < 0)
        return -1;
    if (mkdir(rp, 0777) == 0 || errno == EEXIST)
        return 0;
    return -1;
}

static int h_remove(void *c, const char *path)
{
    (void)c;
    char rp[2048];
    if (real_path(rp, sizeof rp, path) < 0)
        return -1;
    if (remove(rp) == 0)
        return 0;
    return rmdir(rp) == 0 ? 0 : -1;
}

static void make_parents(char *rp)
{
    for (char *p = rp + strlen(g_root) + 1; *p; p++) {
        if (*p != '/')
            continue;
        *p = 0;
        mkdir(rp, 0777);
        *p = '/';
    }
}

static int h_put_begin(void *c, const char *path, uint32_t size)
{
    (void)c; (void)size;
    if (g_put) { fclose(g_put); g_put = NULL; remove(g_put_tmp); }
    if (real_path(g_put_target, sizeof g_put_target, path) < 0)
        return -1;
    snprintf(g_put_tmp, sizeof g_put_tmp, "%s.tmp", g_put_target);
    make_parents(g_put_target);
    g_put = fopen(g_put_tmp, "wb");
    return g_put ? 0 : -1;
}

static int h_put_data(void *c, const void *buf, size_t n)
{
    (void)c;
    if (!g_put)
        return -1;
    return fwrite(buf, 1, n, g_put) == n ? 0 : -1;
}

static int h_put_end(void *c, bool commit)
{
    (void)c;
    if (!g_put)
        return -1;
    fclose(g_put);
    g_put = NULL;
    if (!commit) { remove(g_put_tmp); return 0; }
    remove(g_put_target);
    return rename(g_put_tmp, g_put_target) == 0 ? 0 : -1;
}

static int h_get(void *c, const char *path, int (*send)(void *, const void *, size_t), void *sctx)
{
    (void)c;
    char rp[2048];
    static unsigned char buf[16384];
    if (real_path(rp, sizeof rp, path) < 0)
        return -1;
    FILE *f = fopen(rp, "rb");
    if (!f)
        return -1;
    int total = 0;
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, f)) > 0) {
        if (send(sctx, buf, n) < 0) { fclose(f); return -1; }
        total += (int)n;
    }
    fclose(f);
    return total;
}

static int h_free(void *c, uint64_t *freeb, uint64_t *total)
{
    (void)c;
    struct statvfs st;
    if (statvfs(g_root, &st) != 0)
        return -1;
    *freeb = (uint64_t)st.f_bavail * st.f_frsize;
    *total = (uint64_t)st.f_blocks * st.f_frsize;
    return 0;
}

static int g_sync_count;
static void h_sync_done(void *c) { (void)c; g_sync_count++; }

static const struct link_fs host_fs = {
    .stat = h_stat, .list = h_list, .mkdir = h_mkdir, .remove = h_remove,
    .put_begin = h_put_begin, .put_data = h_put_data, .put_end = h_put_end,
    .get = h_get, .freespace = h_free, .sync_done = h_sync_done, .ctx = NULL,
};

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
    if (argc > 2)
        snprintf(g_root, sizeof g_root, "%s", argv[2]);
    struct link_io io = { .write = out, .ctx = NULL, .hello = "host-harness",
                          .fs = g_root[0] ? &host_fs : NULL };
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
    fprintf(stderr, "link_host: syncs %d rx %llu tx %llu frames %u crc_errors %u resyncs %u errors %u\n",
            g_sync_count,
            (unsigned long long)l.st.rx_bytes, (unsigned long long)l.st.tx_bytes, l.st.frames,
            l.st.crc_errors, l.st.resyncs, l.st.errors);
    return 0;
}
