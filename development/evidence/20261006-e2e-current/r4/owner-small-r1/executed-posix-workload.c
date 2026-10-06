#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <limits.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#define SECTOR_BYTES 4096ULL
#define SECTOR_HEADER_BYTES 16ULL
#define MAX_WORKERS 8
#define MAX_IO_BYTES (1024ULL * 1024ULL)
#define MAX_TOTAL_BYTES (8ULL * 1024ULL * 1024ULL * 1024ULL)
#define DEFAULT_SEED 0x42ULL
#define AFFINE_A 1664525ULL
#define AFFINE_B 1013904223ULL

struct options {
    const char *action;
    const char *root;
    const char *family;
    const char *barrier;
    const char *target;
    const char *trace_path;
    uint64_t total_bytes;
    uint64_t block_bytes;
    int concurrency;
    uint32_t generation;
    uint64_t seed;
};

struct trace_record {
    uint32_t worker;
    uint64_t op_index;
    uint64_t offset;
    uint64_t requested;
    int64_t returned_value;
    int err;
    int verify_errno;
    uint64_t syscall_count;
    uint64_t short_count;
    int64_t last_return;
    uint64_t start_ns;
    uint64_t end_ns;
};

struct worker_status {
    int worker;
    int open_rc;
    int open_errno;
    int64_t file_size_before;
    int64_t file_size_after;
    uint64_t io_count;
    uint64_t io_bytes;
    uint64_t content_checked_bytes;
    int eof_rc;
    int eof_errno;
    const char *barrier_kind;
    int barrier_rc;
    int barrier_errno;
    int close_rc;
    int close_errno;
    uint64_t start_ns;
    uint64_t end_ns;
};

struct shared {
    struct options opt;
    int dirfd;
    uint64_t file_bytes;
    uint64_t ops_per_worker;
    unsigned char *sector_headers;
    unsigned char payload[MAX_WORKERS][SECTOR_BYTES - SECTOR_HEADER_BYTES];
    struct trace_record *records;
    struct worker_status workers[MAX_WORKERS];
    pthread_barrier_t ready_barrier;
    pthread_barrier_t start_barrier;
    pthread_mutex_t failure_lock;
    int failed;
    char failure[256];
};

struct worker_arg {
    struct shared *shared;
    int worker;
};

struct io_result {
    int64_t total;
    int err;
    uint64_t syscall_count;
    uint64_t short_count;
    int64_t last_return;
};

static uint64_t monotonic_ns(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) {
        perror("clock_gettime");
        exit(2);
    }
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

static void set_failure(struct shared *s, const char *where, int err) {
    pthread_mutex_lock(&s->failure_lock);
    if (!s->failed) {
        s->failed = 1;
        snprintf(s->failure, sizeof(s->failure), "%s: errno=%d:%s", where, err, strerror(err));
    }
    pthread_mutex_unlock(&s->failure_lock);
}

static int has_failed(struct shared *s) {
    pthread_mutex_lock(&s->failure_lock);
    int failed = s->failed;
    pthread_mutex_unlock(&s->failure_lock);
    return failed;
}

static void die(const char *msg) {
    fprintf(stderr, "posix-workload: %s\n", msg);
    exit(2);
}

static void die_errno(const char *msg) {
    fprintf(stderr, "posix-workload: %s: errno=%d:%s\n", msg, errno, strerror(errno));
    exit(2);
}

static bool decimal_text(const char *text) {
    if (!text || !text[0]) return false;
    for (const unsigned char *p = (const unsigned char *)text; *p; ++p) {
        if (*p < '0' || *p > '9') return false;
    }
    return true;
}

static uint64_t parse_u64(const char *text, const char *name) {
    if (!decimal_text(text)) {
        fprintf(stderr, "posix-workload: invalid %s: %s\n", name, text ? text : "(null)");
        exit(2);
    }
    char *end = NULL;
    errno = 0;
    unsigned long long value = strtoull(text, &end, 10);
    if (errno || !end || *end) {
        fprintf(stderr, "posix-workload: invalid %s: %s\n", name, text);
        exit(2);
    }
    return (uint64_t)value;
}

static bool streq(const char *a, const char *b) {
    return a && b && strcmp(a, b) == 0;
}

static bool is_power_of_two(uint64_t value) {
    return value && ((value & (value - 1)) == 0);
}

static void reject_json_unsafe_path(const char *name, const char *path) {
    if (!path || path[0] != '/') die(name);
    for (const unsigned char *p = (const unsigned char *)path; *p; ++p) {
        if (*p < 0x20 || *p == '"' || *p == '\\') die("path contains JSON-unsafe character");
    }
}

static void usage(FILE *out) {
    fprintf(out,
        "usage:\n"
        "  posix-workload --help\n"
        "  posix-workload prepare --root DIR --family FAMILY --total-bytes N --block-bytes N --concurrency 1|8 --generation N --trace FILE\n"
        "  posix-workload run --root DIR --family FAMILY --total-bytes N --block-bytes N --concurrency 1|8 --generation N --barrier read-close|owner-close|ext4-close-compensating-fdatasync|fdatasync|fsync --target owner|ext4 --trace FILE\n"
        "  posix-workload verify --root DIR --family FAMILY --total-bytes N --block-bytes N --concurrency 1|8 --generation N --trace FILE\n"
        "families: seq-read, seq-write, random-read, random-overwrite\n"
        "stdout: one JSON summary. trace: JSON lines header, worker lifecycle records, and one record per I/O operation.\n");
}

static void reject_duplicate(bool *seen, const char *key) {
    if (*seen) {
        fprintf(stderr, "posix-workload: duplicate option: %s\n", key);
        exit(2);
    }
    *seen = true;
}

static void parse_args(int argc, char **argv, struct options *opt) {
    memset(opt, 0, sizeof(*opt));
    opt->seed = DEFAULT_SEED;
    bool seen_root = false, seen_family = false, seen_barrier = false, seen_target = false, seen_trace = false;
    bool seen_total = false, seen_block = false, seen_concurrency = false, seen_generation = false, seen_seed = false;
    if (argc == 2 && streq(argv[1], "--help")) {
        usage(stdout);
        exit(0);
    }
    if (argc < 2) {
        usage(stderr);
        exit(2);
    }
    opt->action = argv[1];
    for (int i = 2; i < argc; ++i) {
        const char *key = argv[i];
        if (i + 1 >= argc) die("missing option value");
        const char *value = argv[++i];
        if (streq(key, "--root")) { reject_duplicate(&seen_root, key); opt->root = value; }
        else if (streq(key, "--family")) { reject_duplicate(&seen_family, key); opt->family = value; }
        else if (streq(key, "--barrier")) { reject_duplicate(&seen_barrier, key); opt->barrier = value; }
        else if (streq(key, "--target")) { reject_duplicate(&seen_target, key); opt->target = value; }
        else if (streq(key, "--trace")) { reject_duplicate(&seen_trace, key); opt->trace_path = value; }
        else if (streq(key, "--total-bytes")) { reject_duplicate(&seen_total, key); opt->total_bytes = parse_u64(value, key); }
        else if (streq(key, "--block-bytes")) { reject_duplicate(&seen_block, key); opt->block_bytes = parse_u64(value, key); }
        else if (streq(key, "--concurrency")) {
            reject_duplicate(&seen_concurrency, key);
            uint64_t c = parse_u64(value, key);
            if (c > INT_MAX) die("--concurrency overflow");
            opt->concurrency = (int)c;
        } else if (streq(key, "--generation")) {
            reject_duplicate(&seen_generation, key);
            uint64_t g = parse_u64(value, key);
            if (g > UINT32_MAX) die("--generation overflow");
            opt->generation = (uint32_t)g;
        } else if (streq(key, "--seed")) { reject_duplicate(&seen_seed, key); opt->seed = parse_u64(value, key); }
        else die("unknown option");
    }
    if (!streq(opt->action, "prepare") && !streq(opt->action, "run") && !streq(opt->action, "verify")) die("bad action");
    if (!seen_root || !seen_family || !seen_trace || !seen_total || !seen_block || !seen_concurrency || !seen_generation) die("missing required option");
    reject_json_unsafe_path("--root must be absolute", opt->root);
    reject_json_unsafe_path("--trace must be absolute", opt->trace_path);
    if (!opt->family || (!streq(opt->family, "seq-read") && !streq(opt->family, "seq-write") && !streq(opt->family, "random-read") && !streq(opt->family, "random-overwrite"))) die("bad --family");
    if (seen_seed && opt->seed != DEFAULT_SEED) die("--seed must remain 0x42 for this fixture contract");
    if (opt->concurrency != 1 && opt->concurrency != 8) die("--concurrency must be 1 or 8");
    if (opt->block_bytes != 4096 && opt->block_bytes != 65536 && opt->block_bytes != 1048576) die("--block-bytes must be 4096, 65536, or 1048576");
    if (opt->block_bytes > MAX_IO_BYTES || opt->block_bytes % SECTOR_BYTES != 0) die("block size unsupported");
    if (opt->total_bytes == 0 || opt->total_bytes > MAX_TOTAL_BYTES) die("--total-bytes must be within 1..8GiB");
    if (opt->total_bytes % (uint64_t)opt->concurrency != 0) die("total bytes must divide concurrency");
    if ((opt->total_bytes / (uint64_t)opt->concurrency) % opt->block_bytes != 0) die("per-worker bytes must divide block size");
    if (!is_power_of_two((opt->total_bytes / (uint64_t)opt->concurrency) / opt->block_bytes)) die("blocks per file must be power of two for affine permutation");
    if (!streq(opt->action, "run") && (opt->barrier || opt->target)) die("prepare/verify do not accept --barrier or --target");
    if (streq(opt->action, "run")) {
        if (!opt->barrier || !opt->target) die("run requires --barrier and --target");
        if (!streq(opt->target, "owner") && !streq(opt->target, "ext4")) die("bad --target");
        if (!streq(opt->barrier, "read-close") && !streq(opt->barrier, "owner-close") && !streq(opt->barrier, "ext4-close-compensating-fdatasync") && !streq(opt->barrier, "fdatasync") && !streq(opt->barrier, "fsync")) die("bad --barrier");
        bool read_family = streq(opt->family, "seq-read") || streq(opt->family, "random-read");
        if (read_family && !streq(opt->barrier, "read-close")) die("read run requires read-close barrier label");
        if (!read_family && streq(opt->barrier, "read-close")) die("read-close barrier is read-only");
        if (!read_family && streq(opt->barrier, "owner-close") && !streq(opt->target, "owner")) die("owner-close requires owner target");
        if (streq(opt->barrier, "ext4-close-compensating-fdatasync") && !streq(opt->target, "ext4")) die("ext4 compensating close requires ext4 target");
    }
}

static int open_dir_fixed(const char *path, bool create) {
    if (create && mkdir(path, 0700) != 0 && errno != EEXIST) die_errno("mkdir root");
    int fd = open(path, O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW);
    if (fd < 0) die_errno("open root dir");
    return fd;
}

static void file_name(char *buf, size_t n, int worker) {
    if (snprintf(buf, n, "worker-%02d.dat", worker) >= (int)n) die("file name too long");
}

static void write_le32(unsigned char *p, uint32_t v) {
    p[0] = (unsigned char)(v & 0xff);
    p[1] = (unsigned char)((v >> 8) & 0xff);
    p[2] = (unsigned char)((v >> 16) & 0xff);
    p[3] = (unsigned char)((v >> 24) & 0xff);
}

static void write_le64(unsigned char *p, uint64_t v) {
    for (int i = 0; i < 8; ++i) p[i] = (unsigned char)((v >> (8 * i)) & 0xff);
}

static uint64_t splitmix64(uint64_t x) {
    x += 0x9e3779b97f4a7c15ULL;
    x = (x ^ (x >> 30)) * 0xbf58476d1ce4e5b9ULL;
    x = (x ^ (x >> 27)) * 0x94d049bb133111ebULL;
    return x ^ (x >> 31);
}

static void build_patterns(struct shared *s) {
    uint64_t sector_count = s->file_bytes / SECTOR_BYTES;
    s->sector_headers = calloc((size_t)sector_count, (size_t)SECTOR_HEADER_BYTES);
    if (!s->sector_headers) die("sector header allocation");
    for (uint64_t sector = 0; sector < sector_count; ++sector) {
        unsigned char *h = s->sector_headers + sector * SECTOR_HEADER_BYTES;
        write_le32(h, 0);
        write_le32(h + 4, s->opt.generation);
        write_le64(h + 8, sector * SECTOR_BYTES);
    }
    for (int worker = 0; worker < s->opt.concurrency; ++worker) {
        for (uint64_t i = 0; i < SECTOR_BYTES - SECTOR_HEADER_BYTES; ++i) {
            uint64_t mixed = splitmix64(s->opt.seed ^ ((uint64_t)s->opt.generation << 32) ^ ((uint64_t)worker << 48) ^ i);
            s->payload[worker][i] = (unsigned char)(mixed & 0xff);
        }
    }
}

static void fill_buffer(struct shared *s, int worker, uint64_t offset, unsigned char *buf, uint64_t len) {
    uint64_t done = 0;
    while (done < len) {
        uint64_t absolute = offset + done;
        uint64_t sector = absolute / SECTOR_BYTES;
        uint64_t in_sector = absolute % SECTOR_BYTES;
        uint64_t take = SECTOR_BYTES - in_sector;
        if (take > len - done) take = len - done;
        unsigned char tmp[SECTOR_BYTES];
        memcpy(tmp, s->sector_headers + sector * SECTOR_HEADER_BYTES, SECTOR_HEADER_BYTES);
        write_le32(tmp, (uint32_t)worker);
        memcpy(tmp + SECTOR_HEADER_BYTES, s->payload[worker], SECTOR_BYTES - SECTOR_HEADER_BYTES);
        memcpy(buf + done, tmp + in_sector, (size_t)take);
        done += take;
    }
}

static int verify_buffer(struct shared *s, int worker, uint64_t offset, const unsigned char *buf, uint64_t len) {
    uint64_t done = 0;
    while (done < len) {
        uint64_t absolute = offset + done;
        uint64_t sector = absolute / SECTOR_BYTES;
        uint64_t in_sector = absolute % SECTOR_BYTES;
        uint64_t take = SECTOR_BYTES - in_sector;
        if (take > len - done) take = len - done;
        unsigned char tmp[SECTOR_BYTES];
        memcpy(tmp, s->sector_headers + sector * SECTOR_HEADER_BYTES, SECTOR_HEADER_BYTES);
        write_le32(tmp, (uint32_t)worker);
        memcpy(tmp + SECTOR_HEADER_BYTES, s->payload[worker], SECTOR_BYTES - SECTOR_HEADER_BYTES);
        if (memcmp(buf + done, tmp + in_sector, (size_t)take) != 0) return EIO;
        done += take;
    }
    return 0;
}

static struct io_result full_pread_recorded(int fd, void *buf, size_t count, uint64_t offset) {
    struct io_result r = {0, 0, 0, 0, -2};
    while ((size_t)r.total < count) {
        size_t remain = count - (size_t)r.total;
        errno = 0;
        ssize_t n = pread(fd, (char *)buf + r.total, remain, (off_t)(offset + (uint64_t)r.total));
        r.syscall_count++;
        r.last_return = n;
        if (n < 0) {
            r.err = errno ? errno : EIO;
            return r;
        }
        if (n == 0) return r;
        if ((size_t)n < remain) r.short_count++;
        r.total += n;
    }
    return r;
}

static struct io_result full_pwrite_recorded(int fd, const void *buf, size_t count, uint64_t offset) {
    struct io_result r = {0, 0, 0, 0, -2};
    while ((size_t)r.total < count) {
        size_t remain = count - (size_t)r.total;
        errno = 0;
        ssize_t n = pwrite(fd, (const char *)buf + r.total, remain, (off_t)(offset + (uint64_t)r.total));
        r.syscall_count++;
        r.last_return = n;
        if (n < 0) {
            r.err = errno ? errno : EIO;
            return r;
        }
        if (n == 0) {
            r.err = EIO;
            return r;
        }
        if ((size_t)n < remain) r.short_count++;
        r.total += n;
    }
    return r;
}

static uint64_t op_offset(struct shared *s, uint64_t j) {
    uint64_t blocks = s->file_bytes / s->opt.block_bytes;
    if (streq(s->opt.family, "seq-read") || streq(s->opt.family, "seq-write")) return j * s->opt.block_bytes;
    uint64_t idx = (AFFINE_A * j + AFFINE_B + s->opt.seed) & (blocks - 1);
    return idx * s->opt.block_bytes;
}

static int open_worker_file(struct shared *s, int worker, int flags) {
    char name[64];
    file_name(name, sizeof(name), worker);
    return openat(s->dirfd, name, flags | O_CLOEXEC | O_NOFOLLOW, 0600);
}

static int64_t fd_size(int fd, int *err) {
    struct stat st;
    if (fstat(fd, &st) != 0) {
        *err = errno;
        return -1;
    }
    *err = 0;
    return (int64_t)st.st_size;
}

static int eof_check(int fd, uint64_t file_bytes) {
    unsigned char one = 0;
    errno = 0;
    ssize_t n = pread(fd, &one, 1, (off_t)file_bytes);
    if (n == 0) return 0;
    if (n < 0) return errno ? errno : EIO;
    return EFBIG;
}

static int fresh_verify_worker(struct shared *s, int worker) {
    int fd = open_worker_file(s, worker, O_RDONLY);
    if (fd < 0) return errno;
    int size_err = 0;
    int64_t size = fd_size(fd, &size_err);
    if (size_err) {
        close(fd);
        return size_err;
    }
    if (size != (int64_t)s->file_bytes) {
        close(fd);
        return EINVAL;
    }
    unsigned char *buf = malloc((size_t)s->opt.block_bytes);
    if (!buf) {
        close(fd);
        return ENOMEM;
    }
    for (uint64_t j = 0; j < s->ops_per_worker; ++j) {
        uint64_t offset = j * s->opt.block_bytes;
        struct io_result io = full_pread_recorded(fd, buf, (size_t)s->opt.block_bytes, offset);
        if (io.total != (int64_t)s->opt.block_bytes || io.err) {
            int err = io.err ? io.err : EIO;
            free(buf);
            close(fd);
            return err;
        }
        int err = verify_buffer(s, worker, offset, buf, s->opt.block_bytes);
        if (err) {
            free(buf);
            close(fd);
            return err;
        }
    }
    int eof_err = eof_check(fd, s->file_bytes);
    free(buf);
    if (close(fd) != 0 && eof_err == 0) eof_err = errno;
    return eof_err;
}

static int do_worker_barrier(struct shared *s, int fd) {
    if (streq(s->opt.barrier, "owner-close") || streq(s->opt.barrier, "read-close")) return 0;
    if (streq(s->opt.barrier, "ext4-close-compensating-fdatasync") || streq(s->opt.barrier, "fdatasync")) return fdatasync(fd) == 0 ? 0 : errno;
    if (streq(s->opt.barrier, "fsync")) return fsync(fd) == 0 ? 0 : errno;
    return EINVAL;
}

static void *worker_main(void *opaque) {
    struct worker_arg *arg = (struct worker_arg *)opaque;
    struct shared *s = arg->shared;
    int worker = arg->worker;
    struct worker_status *ws = &s->workers[worker];
    bool write_mode = streq(s->opt.family, "seq-write") || streq(s->opt.family, "random-overwrite");
    int flags = write_mode ? O_RDWR : O_RDONLY;
    if (streq(s->opt.family, "seq-write")) flags |= O_CREAT | O_EXCL;
    ws->worker = worker;
    ws->open_rc = -1;
    ws->file_size_before = -1;
    ws->file_size_after = -1;
    ws->barrier_kind = s->opt.barrier ? s->opt.barrier : "none";
    ws->barrier_rc = 0;
    ws->close_rc = -1;
    unsigned char *buf = malloc((size_t)s->opt.block_bytes);
    if (!buf) {
        set_failure(s, "buffer allocation", ENOMEM);
        pthread_barrier_wait(&s->ready_barrier);
        pthread_barrier_wait(&s->start_barrier);
        ws->start_ns = monotonic_ns();
        ws->end_ns = ws->start_ns;
        return NULL;
    }
    pthread_barrier_wait(&s->ready_barrier);
    pthread_barrier_wait(&s->start_barrier);
    ws->start_ns = monotonic_ns();
    int fd = open_worker_file(s, worker, flags);
    if (fd < 0) {
        ws->open_errno = errno;
        set_failure(s, "openat", errno);
        ws->end_ns = monotonic_ns();
        free(buf);
        return NULL;
    }
    ws->open_rc = 0;
    int size_err = 0;
    ws->file_size_before = fd_size(fd, &size_err);
    if (size_err) {
        set_failure(s, "fstat before", size_err);
    } else if (!streq(s->opt.family, "seq-write") && ws->file_size_before != (int64_t)s->file_bytes) {
        set_failure(s, "unexpected file size before", EINVAL);
    }
    for (uint64_t j = 0; j < s->ops_per_worker && !has_failed(s); ++j) {
        uint64_t offset = op_offset(s, j);
        uint64_t global = (uint64_t)worker * s->ops_per_worker + j;
        struct trace_record *rec = &s->records[global];
        rec->worker = (uint32_t)worker;
        rec->op_index = j;
        rec->offset = offset;
        rec->requested = s->opt.block_bytes;
        rec->start_ns = monotonic_ns();
        if (write_mode) {
            fill_buffer(s, worker, offset, buf, s->opt.block_bytes);
            struct io_result io = full_pwrite_recorded(fd, buf, (size_t)s->opt.block_bytes, offset);
            rec->returned_value = io.total;
            rec->err = io.err;
            rec->syscall_count = io.syscall_count;
            rec->short_count = io.short_count;
            rec->last_return = io.last_return;
        } else {
            memset(buf, 0, (size_t)s->opt.block_bytes);
            struct io_result io = full_pread_recorded(fd, buf, (size_t)s->opt.block_bytes, offset);
            rec->returned_value = io.total;
            rec->err = io.err;
            rec->syscall_count = io.syscall_count;
            rec->short_count = io.short_count;
            rec->last_return = io.last_return;
            if (rec->returned_value == (int64_t)s->opt.block_bytes && rec->err == 0) {
                int err = verify_buffer(s, worker, offset, buf, s->opt.block_bytes);
                rec->verify_errno = err;
                if (!err) ws->content_checked_bytes += s->opt.block_bytes;
            }
        }
        rec->end_ns = monotonic_ns();
        if (rec->returned_value == (int64_t)s->opt.block_bytes && rec->err == 0 && rec->verify_errno == 0) {
            ws->io_count++;
            ws->io_bytes += s->opt.block_bytes;
        } else {
            int fail_err = rec->err ? rec->err : (rec->verify_errno ? rec->verify_errno : EIO);
            set_failure(s, write_mode ? "io-write" : "io-read", fail_err);
            break;
        }
    }
    ws->file_size_after = fd_size(fd, &size_err);
    if (size_err) set_failure(s, "fstat after", size_err);
    else if (ws->file_size_after != (int64_t)s->file_bytes) set_failure(s, "unexpected file size after", EINVAL);
    if (!write_mode && !has_failed(s)) {
        int err = eof_check(fd, s->file_bytes);
        ws->eof_rc = err == 0 ? 0 : -1;
        ws->eof_errno = err;
        if (err) set_failure(s, "eof check", err);
    }
    if (write_mode && !has_failed(s)) {
        int err = do_worker_barrier(s, fd);
        ws->barrier_rc = err == 0 ? 0 : -1;
        ws->barrier_errno = err;
        if (err) set_failure(s, "worker barrier", err);
    }
    if (close(fd) != 0) {
        ws->close_rc = -1;
        ws->close_errno = errno;
        set_failure(s, "worker close", errno);
    } else {
        ws->close_rc = 0;
        ws->close_errno = 0;
    }
    ws->end_ns = monotonic_ns();
    free(buf);
    return NULL;
}

static void fresh_verify_all(struct shared *s) {
    for (int worker = 0; worker < s->opt.concurrency; ++worker) {
        int err = fresh_verify_worker(s, worker);
        if (err) {
            set_failure(s, "fresh verify", err);
            return;
        }
        s->workers[worker].content_checked_bytes += s->file_bytes;
    }
}

static void write_worker_json(FILE *out, const struct worker_status *w, bool bare) {
    fprintf(out, "%s{\"record_type\":\"worker\",\"worker\":%d,\"open_rc\":%d,\"open_errno\":%d,\"file_size_before\":%" PRId64 ",\"file_size_after\":%" PRId64 ",\"io_count\":%" PRIu64 ",\"io_bytes\":%" PRIu64 ",\"content_checked_bytes\":%" PRIu64 ",\"eof_rc\":%d,\"eof_errno\":%d,\"barrier_kind\":\"%s\",\"barrier_rc\":%d,\"barrier_errno\":%d,\"close_rc\":%d,\"close_errno\":%d,\"start_ns\":%" PRIu64 ",\"end_ns\":%" PRIu64 "}%s",
            bare ? "" : "", w->worker, w->open_rc, w->open_errno, w->file_size_before, w->file_size_after, w->io_count, w->io_bytes, w->content_checked_bytes, w->eof_rc, w->eof_errno, w->barrier_kind ? w->barrier_kind : "none", w->barrier_rc, w->barrier_errno, w->close_rc, w->close_errno, w->start_ns, w->end_ns, bare ? "" : "\n");
}

static uint64_t max_worker_end_ns(struct shared *s, uint64_t fallback) {
    uint64_t out = fallback;
    for (int i = 0; i < s->opt.concurrency; ++i) {
        if (s->workers[i].end_ns > out) out = s->workers[i].end_ns;
    }
    return out;
}

static void write_trace(struct shared *s, const char *status, uint64_t ready_ns, uint64_t task_start_ns, uint64_t task_end_ns, uint64_t join_end_ns, uint64_t dir_barrier_ns, int dir_barrier_rc, int dir_barrier_errno, uint64_t record_count) {
    int fd = open(s->opt.trace_path, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
    if (fd < 0) die_errno("create trace");
    FILE *out = fdopen(fd, "w");
    if (!out) die_errno("fdopen trace");
    uint64_t wall_ns = task_end_ns >= task_start_ns ? task_end_ns - task_start_ns : 0;
    fprintf(out, "{\"schema\":\"g2-ordinary-posix-workload-trace-r1\",\"record_type\":\"header\",\"status\":\"%s\",\"action\":\"%s\",\"family\":\"%s\",\"target\":\"%s\",\"barrier\":\"%s\",\"root\":\"%s\",\"total_bytes\":%" PRIu64 ",\"file_bytes\":%" PRIu64 ",\"block_bytes\":%" PRIu64 ",\"concurrency\":%d,\"generation\":%u,\"seed\":%" PRIu64 ",\"ready_ns\":%" PRIu64 ",\"task_start_ns\":%" PRIu64 ",\"task_end_ns\":%" PRIu64 ",\"join_end_ns\":%" PRIu64 ",\"wall_ns\":%" PRIu64 ",\"dir_barrier_ns\":%" PRIu64 ",\"dir_barrier_rc\":%d,\"dir_barrier_errno\":%d}\n",
            status, s->opt.action, s->opt.family, s->opt.target ? s->opt.target : "none", s->opt.barrier ? s->opt.barrier : "none", s->opt.root, s->opt.total_bytes, s->file_bytes, s->opt.block_bytes, s->opt.concurrency, s->opt.generation, s->opt.seed, ready_ns, task_start_ns, task_end_ns, join_end_ns, wall_ns, dir_barrier_ns, dir_barrier_rc, dir_barrier_errno);
    for (int i = 0; i < s->opt.concurrency; ++i) write_worker_json(out, &s->workers[i], false);
    for (uint64_t i = 0; i < record_count; ++i) {
        struct trace_record *r = &s->records[i];
        fprintf(out, "{\"record_type\":\"io\",\"worker\":%u,\"op_index\":%" PRIu64 ",\"offset\":%" PRIu64 ",\"requested\":%" PRIu64 ",\"return\":%" PRId64 ",\"errno\":%d,\"verify_errno\":%d,\"syscall_count\":%" PRIu64 ",\"short_count\":%" PRIu64 ",\"last_return\":%" PRId64 ",\"start_ns\":%" PRIu64 ",\"end_ns\":%" PRIu64 "}\n", r->worker, r->op_index, r->offset, r->requested, r->returned_value, r->err, r->verify_errno, r->syscall_count, r->short_count, r->last_return, r->start_ns, r->end_ns);
    }
    if (fclose(out) != 0) die_errno("close trace");
}

static void init_shared(struct shared *s, const struct options *opt) {
    memset(s, 0, sizeof(*s));
    s->opt = *opt;
    s->file_bytes = opt->total_bytes / (uint64_t)opt->concurrency;
    s->ops_per_worker = s->file_bytes / opt->block_bytes;
    s->dirfd = open_dir_fixed(opt->root, streq(opt->action, "prepare"));
    uint64_t total_ops = s->ops_per_worker * (uint64_t)opt->concurrency;
    s->records = calloc((size_t)total_ops, sizeof(*s->records));
    if (!s->records) die("trace record allocation");
    if (pthread_mutex_init(&s->failure_lock, NULL) != 0) die("failure mutex init");
    for (int i = 0; i < MAX_WORKERS; ++i) {
        s->workers[i].worker = i;
        s->workers[i].open_rc = -1;
        s->workers[i].file_size_before = -1;
        s->workers[i].file_size_after = -1;
        s->workers[i].barrier_kind = "none";
        s->workers[i].close_rc = -1;
    }
    build_patterns(s);
}

static int dir_fsync_timed(struct shared *s, uint64_t *dir_ns, int *dir_rc, int *dir_errno) {
    uint64_t begin = monotonic_ns();
    *dir_rc = fsync(s->dirfd) == 0 ? 0 : -1;
    *dir_errno = *dir_rc == 0 ? 0 : errno;
    *dir_ns = monotonic_ns() - begin;
    return *dir_rc;
}

static void fill_record_from_io(struct trace_record *rec, struct io_result io) {
    rec->returned_value = io.total;
    rec->err = io.err;
    rec->syscall_count = io.syscall_count;
    rec->short_count = io.short_count;
    rec->last_return = io.last_return;
}

static int do_prepare_or_verify(struct shared *s, bool prepare) {
    uint64_t wall_begin = monotonic_ns();
    if (prepare && streq(s->opt.family, "seq-write")) {
        uint64_t dir_ns = 0;
        int dir_rc = 0;
        int dir_errno = 0;
        if (dir_fsync_timed(s, &dir_ns, &dir_rc, &dir_errno) != 0) die_errno("prepare seq-write dir fsync");
        uint64_t task_end = monotonic_ns();
        uint64_t wall_ns = task_end - wall_begin;
        write_trace(s, "PASS", 0, wall_begin, task_end, task_end, dir_ns, dir_rc, dir_errno, 0);
        printf("{\"schema\":\"g2-ordinary-posix-workload-summary-r1\",\"status\":\"PASS\",\"action\":\"prepare\",\"family\":\"%s\",\"total_bytes\":%" PRIu64 ",\"file_bytes\":%" PRIu64 ",\"block_bytes\":%" PRIu64 ",\"concurrency\":%d,\"generation\":%u,\"operations\":0,\"ready_ns\":0,\"task_start_ns\":%" PRIu64 ",\"task_end_ns\":%" PRIu64 ",\"join_end_ns\":%" PRIu64 ",\"wall_ns\":%" PRIu64 ",\"dir_barrier_ns\":%" PRIu64 ",\"dir_barrier_rc\":%d,\"dir_barrier_errno\":%d,\"trace_path\":\"%s\",\"note\":\"fresh-create family prepared directory only\",\"workers\":[",
               s->opt.family, s->opt.total_bytes, s->file_bytes, s->opt.block_bytes, s->opt.concurrency, s->opt.generation, wall_begin, task_end, task_end, wall_ns, dir_ns, dir_rc, dir_errno, s->opt.trace_path);
        for (int i = 0; i < s->opt.concurrency; ++i) {
            if (i) printf(",");
            write_worker_json(stdout, &s->workers[i], true);
        }
        printf("]}\n");
        return 0;
    }
    for (int worker = 0; worker < s->opt.concurrency; ++worker) {
        struct worker_status *ws = &s->workers[worker];
        ws->worker = worker;
        ws->barrier_kind = prepare ? "prepare-fdatasync" : "verify-read-close";
        ws->start_ns = monotonic_ns();
        int flags = prepare ? (O_RDWR | O_CREAT | O_EXCL) : O_RDONLY;
        int fd = open_worker_file(s, worker, flags);
        if (fd < 0) die_errno(prepare ? "prepare openat" : "verify openat");
        ws->open_rc = 0;
        int size_err = 0;
        ws->file_size_before = fd_size(fd, &size_err);
        if (size_err) { errno = size_err; die_errno("prepare/verify fstat before"); }
        unsigned char *buf = malloc((size_t)s->opt.block_bytes);
        if (!buf) die("prepare/verify buffer allocation");
        uint64_t ops = s->file_bytes / s->opt.block_bytes;
        for (uint64_t j = 0; j < ops; ++j) {
            uint64_t offset = j * s->opt.block_bytes;
            uint64_t global = (uint64_t)worker * ops + j;
            struct trace_record *rec = &s->records[global];
            rec->worker = (uint32_t)worker;
            rec->op_index = j;
            rec->offset = offset;
            rec->requested = s->opt.block_bytes;
            rec->start_ns = monotonic_ns();
            if (prepare) {
                fill_buffer(s, worker, offset, buf, s->opt.block_bytes);
                fill_record_from_io(rec, full_pwrite_recorded(fd, buf, (size_t)s->opt.block_bytes, offset));
            } else {
                fill_record_from_io(rec, full_pread_recorded(fd, buf, (size_t)s->opt.block_bytes, offset));
                if (rec->returned_value == (int64_t)s->opt.block_bytes && rec->err == 0) {
                    rec->verify_errno = verify_buffer(s, worker, offset, buf, s->opt.block_bytes);
                    if (!rec->verify_errno) ws->content_checked_bytes += s->opt.block_bytes;
                }
            }
            rec->end_ns = monotonic_ns();
            if (rec->returned_value != (int64_t)s->opt.block_bytes || rec->err != 0 || rec->verify_errno != 0) {
                int fail_err = rec->err ? rec->err : (rec->verify_errno ? rec->verify_errno : EIO);
                errno = fail_err;
                die_errno(prepare ? "prepare write" : "verify read");
            }
            ws->io_count++;
            ws->io_bytes += s->opt.block_bytes;
        }
        ws->file_size_after = fd_size(fd, &size_err);
        if (size_err) { errno = size_err; die_errno("prepare/verify fstat after"); }
        if (ws->file_size_after != (int64_t)s->file_bytes) die("prepare/verify unexpected file size");
        int eof_err = eof_check(fd, s->file_bytes);
        ws->eof_rc = eof_err == 0 ? 0 : -1;
        ws->eof_errno = eof_err;
        if (eof_err) { errno = eof_err; die_errno("prepare/verify eof check"); }
        if (prepare) {
            int err = fdatasync(fd) == 0 ? 0 : errno;
            ws->barrier_rc = err == 0 ? 0 : -1;
            ws->barrier_errno = err;
            if (err) { errno = err; die_errno("prepare fdatasync"); }
        }
        if (close(fd) != 0) die_errno("prepare/verify close");
        ws->close_rc = 0;
        ws->close_errno = 0;
        ws->end_ns = monotonic_ns();
        free(buf);
    }
    uint64_t dir_ns = 0;
    int dir_rc = 0;
    int dir_errno = 0;
    if (dir_fsync_timed(s, &dir_ns, &dir_rc, &dir_errno) != 0) die_errno("prepare/verify dir fsync");
    uint64_t records = s->ops_per_worker * (uint64_t)s->opt.concurrency;
    if (prepare) fresh_verify_all(s);
    if (has_failed(s)) {
        uint64_t task_end = monotonic_ns();
        write_trace(s, "FAIL", 0, wall_begin, task_end, task_end, dir_ns, dir_rc, dir_errno, records);
        fprintf(stderr, "posix-workload: %s\n", s->failure);
        return 1;
    }
    uint64_t task_end = monotonic_ns();
    uint64_t wall_ns = task_end - wall_begin;
    write_trace(s, "PASS", 0, wall_begin, task_end, task_end, dir_ns, dir_rc, dir_errno, records);
    printf("{\"schema\":\"g2-ordinary-posix-workload-summary-r1\",\"status\":\"PASS\",\"action\":\"%s\",\"family\":\"%s\",\"total_bytes\":%" PRIu64 ",\"file_bytes\":%" PRIu64 ",\"block_bytes\":%" PRIu64 ",\"concurrency\":%d,\"generation\":%u,\"operations\":%" PRIu64 ",\"ready_ns\":0,\"task_start_ns\":%" PRIu64 ",\"task_end_ns\":%" PRIu64 ",\"join_end_ns\":%" PRIu64 ",\"wall_ns\":%" PRIu64 ",\"dir_barrier_ns\":%" PRIu64 ",\"dir_barrier_rc\":%d,\"dir_barrier_errno\":%d,\"trace_path\":\"%s\",\"workers\":[",
           s->opt.action, s->opt.family, s->opt.total_bytes, s->file_bytes, s->opt.block_bytes, s->opt.concurrency, s->opt.generation, records, wall_begin, task_end, task_end, wall_ns, dir_ns, dir_rc, dir_errno, s->opt.trace_path);
    for (int i = 0; i < s->opt.concurrency; ++i) {
        if (i) printf(",");
        write_worker_json(stdout, &s->workers[i], true);
    }
    printf("]}\n");
    return 0;
}

static int do_run(struct shared *s) {
    if (pthread_barrier_init(&s->ready_barrier, NULL, (unsigned)s->opt.concurrency + 1) != 0) die("ready barrier init");
    if (pthread_barrier_init(&s->start_barrier, NULL, (unsigned)s->opt.concurrency + 1) != 0) die("start barrier init");
    pthread_t threads[MAX_WORKERS];
    struct worker_arg args[MAX_WORKERS];
    for (int i = 0; i < s->opt.concurrency; ++i) {
        args[i].shared = s;
        args[i].worker = i;
        int rc = pthread_create(&threads[i], NULL, worker_main, &args[i]);
        if (rc != 0) die("pthread_create");
    }
    pthread_barrier_wait(&s->ready_barrier);
    uint64_t ready_ns = monotonic_ns();
    uint64_t begin = monotonic_ns();
    pthread_barrier_wait(&s->start_barrier);
    for (int i = 0; i < s->opt.concurrency; ++i) {
        if (pthread_join(threads[i], NULL) != 0) die("pthread_join");
    }
    uint64_t join_end_ns = monotonic_ns();
    uint64_t task_end_ns = max_worker_end_ns(s, begin);
    uint64_t wall_ns = task_end_ns >= begin ? task_end_ns - begin : 0;
    uint64_t dir_ns = 0;
    int dir_rc = 0;
    int dir_errno = 0;
    if (dir_fsync_timed(s, &dir_ns, &dir_rc, &dir_errno) != 0) set_failure(s, "run dir fsync", dir_errno);
    bool write_mode = streq(s->opt.family, "seq-write") || streq(s->opt.family, "random-overwrite");
    if (write_mode && !has_failed(s)) fresh_verify_all(s);
    const char *status = has_failed(s) ? "FAIL" : "PASS";
    uint64_t operations = s->ops_per_worker * (uint64_t)s->opt.concurrency;
    write_trace(s, status, ready_ns, begin, task_end_ns, join_end_ns, dir_ns, dir_rc, dir_errno, operations);
    if (has_failed(s)) {
        fprintf(stderr, "posix-workload: %s\n", s->failure);
        printf("{\"schema\":\"g2-ordinary-posix-workload-summary-r1\",\"status\":\"FAIL\",\"failure\":\"%s\",\"trace_path\":\"%s\",\"ready_ns\":%" PRIu64 ",\"task_start_ns\":%" PRIu64 ",\"task_end_ns\":%" PRIu64 ",\"join_end_ns\":%" PRIu64 ",\"wall_ns\":%" PRIu64 ",\"dir_barrier_ns\":%" PRIu64 ",\"dir_barrier_rc\":%d,\"dir_barrier_errno\":%d,\"workers\":[", s->failure, s->opt.trace_path, ready_ns, begin, task_end_ns, join_end_ns, wall_ns, dir_ns, dir_rc, dir_errno);
        for (int i = 0; i < s->opt.concurrency; ++i) {
            if (i) printf(",");
            write_worker_json(stdout, &s->workers[i], true);
        }
        printf("]}\n");
        return 1;
    }
    printf("{\"schema\":\"g2-ordinary-posix-workload-summary-r1\",\"status\":\"PASS\",\"action\":\"run\",\"family\":\"%s\",\"target\":\"%s\",\"barrier\":\"%s\",\"total_bytes\":%" PRIu64 ",\"file_bytes\":%" PRIu64 ",\"block_bytes\":%" PRIu64 ",\"concurrency\":%d,\"generation\":%u,\"operations\":%" PRIu64 ",\"ready_ns\":%" PRIu64 ",\"task_start_ns\":%" PRIu64 ",\"task_end_ns\":%" PRIu64 ",\"join_end_ns\":%" PRIu64 ",\"wall_ns\":%" PRIu64 ",\"dir_barrier_ns\":%" PRIu64 ",\"dir_barrier_rc\":%d,\"dir_barrier_errno\":%d,\"trace_path\":\"%s\",\"workers\":[",
           s->opt.family, s->opt.target, s->opt.barrier, s->opt.total_bytes, s->file_bytes, s->opt.block_bytes, s->opt.concurrency, s->opt.generation, operations, ready_ns, begin, task_end_ns, join_end_ns, wall_ns, dir_ns, dir_rc, dir_errno, s->opt.trace_path);
    for (int i = 0; i < s->opt.concurrency; ++i) {
        if (i) printf(",");
        write_worker_json(stdout, &s->workers[i], true);
    }
    printf("]}\n");
    return 0;
}

int main(int argc, char **argv) {
    struct options opt;
    parse_args(argc, argv, &opt);
    struct shared s;
    init_shared(&s, &opt);
    if (streq(opt.action, "prepare")) return do_prepare_or_verify(&s, true);
    if (streq(opt.action, "verify")) return do_prepare_or_verify(&s, false);
    return do_run(&s);
}
