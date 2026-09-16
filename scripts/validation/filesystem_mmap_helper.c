#define _GNU_SOURCE
#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <fcntl.h>
#include <setjmp.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

/*
 * Native mmap verifier used by the Python M1.6b harness.
 *
 * Python can orchestrate mounts and metrics, but it cannot reliably assert
 * SIGBUS-on-access or keep a mapped page alive while another path mutates the
 * same file.  This helper keeps those checks in plain POSIX C and prints one
 * JSON object per subcommand.  Any failed invariant exits non-zero.
 */

static sigjmp_buf sigbus_jump;
static volatile sig_atomic_t saw_sigbus = 0;

static void handle_sigbus(int signal_number) {
    (void)signal_number;
    saw_sigbus = 1;
    siglongjmp(sigbus_jump, 1);
}

static int fail_errno(const char *operation) {
    fprintf(stderr, "%s: %s\n", operation, strerror(errno));
    return 2;
}

static int fail_msg(const char *message) {
    fprintf(stderr, "%s\n", message);
    return 3;
}

static int write_full(int fd, const void *buffer, size_t length) {
    const uint8_t *cursor = (const uint8_t *)buffer;
    while (length > 0) {
        ssize_t written = write(fd, cursor, length);
        if (written < 0) {
            if (errno == EINTR) {
                continue;
            }
            return -1;
        }
        cursor += (size_t)written;
        length -= (size_t)written;
    }
    return 0;
}

static int read_exact_at(int fd, off_t offset, void *buffer, size_t length) {
    uint8_t *cursor = (uint8_t *)buffer;
    while (length > 0) {
        ssize_t count = pread(fd, cursor, length, offset);
        if (count < 0) {
            if (errno == EINTR) {
                continue;
            }
            return -1;
        }
        if (count == 0) {
            return 1;
        }
        cursor += (size_t)count;
        offset += count;
        length -= (size_t)count;
    }
    return 0;
}

static long elapsed_us(struct timespec start, struct timespec end) {
    int64_t seconds = (int64_t)end.tv_sec - (int64_t)start.tv_sec;
    int64_t nanos = (int64_t)end.tv_nsec - (int64_t)start.tv_nsec;
    return (long)((seconds * 1000000000LL + nanos) / 1000LL);
}

static int create_pattern_file(const char *path, size_t length, uint8_t value) {
    int fd = open(path, O_CREAT | O_TRUNC | O_RDWR, 0644);
    if (fd < 0) {
        return -1;
    }
    uint8_t buffer[4096];
    memset(buffer, value, sizeof(buffer));
    size_t remaining = length;
    while (remaining > 0) {
        size_t chunk = remaining < sizeof(buffer) ? remaining : sizeof(buffer);
        if (write_full(fd, buffer, chunk) != 0) {
            close(fd);
            return -1;
        }
        remaining -= chunk;
    }
    if (fsync(fd) != 0) {
        close(fd);
        return -1;
    }
    if (close(fd) != 0) {
        return -1;
    }
    return 0;
}

static int cmd_shared_msync(const char *writer_path, const char *reader_path) {
    if (create_pattern_file(writer_path, 8192, 'A') != 0) {
        return fail_errno("create shared file");
    }
    int fd = open(writer_path, O_RDWR);
    if (fd < 0) {
        return fail_errno("open shared file");
    }
    uint8_t *map = mmap(NULL, 8192, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) {
        close(fd);
        return fail_errno("mmap MAP_SHARED");
    }
    struct timespec start, end;
    clock_gettime(CLOCK_MONOTONIC, &start);
    memcpy(map + 128, "MAP_SHARED_OK", 13);
    if (msync(map, 8192, MS_SYNC) != 0) {
        munmap(map, 8192);
        close(fd);
        return fail_errno("msync MAP_SHARED");
    }
    clock_gettime(CLOCK_MONOTONIC, &end);
    if (munmap(map, 8192) != 0) {
        close(fd);
        return fail_errno("munmap shared");
    }
    if (close(fd) != 0) {
        return fail_errno("close shared");
    }
    fd = open(reader_path, O_RDONLY);
    if (fd < 0) {
        return fail_errno("open reader shared");
    }
    char actual[13];
    if (read_exact_at(fd, 128, actual, sizeof(actual)) != 0) {
        close(fd);
        return fail_errno("pread reader shared");
    }
    close(fd);
    if (memcmp(actual, "MAP_SHARED_OK", sizeof(actual)) != 0) {
        return fail_msg("MAP_SHARED msync was not visible through peer path");
    }
    printf("{\"check\":\"map_shared_msync\",\"status\":\"passed\",\"msync_us\":%ld}\n", elapsed_us(start, end));
    return 0;
}

static int cmd_private_no_publish(const char *path) {
    if (create_pattern_file(path, 4096, 'P') != 0) {
        return fail_errno("create private file");
    }
    int fd = open(path, O_RDWR);
    if (fd < 0) {
        return fail_errno("open private file");
    }
    uint8_t *map = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd, 0);
    if (map == MAP_FAILED) {
        close(fd);
        return fail_errno("mmap MAP_PRIVATE");
    }
    memcpy(map, "PRIVATE_ONLY", 12);
    if (msync(map, 4096, MS_SYNC) != 0) {
        munmap(map, 4096);
        close(fd);
        return fail_errno("msync MAP_PRIVATE");
    }
    if (munmap(map, 4096) != 0) {
        close(fd);
        return fail_errno("munmap private");
    }
    char actual[12];
    if (read_exact_at(fd, 0, actual, sizeof(actual)) != 0) {
        close(fd);
        return fail_errno("pread private");
    }
    close(fd);
    if (memcmp(actual, "PPPPPPPPPPPP", sizeof(actual)) != 0) {
        return fail_msg("MAP_PRIVATE unexpectedly published bytes");
    }
    printf("{\"check\":\"map_private_no_publish\",\"status\":\"passed\"}\n");
    return 0;
}

static int cmd_remote_invalidate(const char *mapped_path, const char *writer_path) {
    if (create_pattern_file(writer_path, 4096, 'O') != 0) {
        return fail_errno("create invalidate file");
    }
    int fd = open(mapped_path, O_RDONLY);
    if (fd < 0) {
        return fail_errno("open mapped path");
    }
    uint8_t *map = mmap(NULL, 4096, PROT_READ, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) {
        close(fd);
        return fail_errno("mmap remote reader");
    }
    volatile uint8_t before = map[0];
    if (before != 'O') {
        munmap(map, 4096);
        close(fd);
        return fail_msg("unexpected initial mapped byte");
    }
    int writer = open(writer_path, O_RDWR);
    if (writer < 0) {
        munmap(map, 4096);
        close(fd);
        return fail_errno("open writer path");
    }
    const char replacement[] = "REMOTE_VERSION";
    if (pwrite(writer, replacement, sizeof(replacement) - 1, 0) != (ssize_t)(sizeof(replacement) - 1)) {
        close(writer);
        munmap(map, 4096);
        close(fd);
        return fail_errno("pwrite remote version");
    }
    if (fsync(writer) != 0) {
        close(writer);
        munmap(map, 4096);
        close(fd);
        return fail_errno("fsync writer");
    }
    close(writer);

    struct timespec start, now;
    clock_gettime(CLOCK_MONOTONIC, &start);
    long waited_us = 0;
    for (;;) {
        if (memcmp((const void *)map, replacement, sizeof(replacement) - 1) == 0) {
            break;
        }
        clock_gettime(CLOCK_MONOTONIC, &now);
        waited_us = elapsed_us(start, now);
        if (waited_us > 5000000) {
            munmap(map, 4096);
            close(fd);
            return fail_msg("mapped page still shows stale bytes after remote write returned");
        }
        struct timespec pause = {.tv_sec = 0, .tv_nsec = 1000000};
        nanosleep(&pause, NULL);
    }
    if (munmap(map, 4096) != 0) {
        close(fd);
        return fail_errno("munmap invalidate");
    }
    close(fd);
    printf("{\"check\":\"remote_invalidate_mapped_page\",\"status\":\"passed\",\"waited_us\":%ld}\n", waited_us);
    return 0;
}

static int wait_for_remote_bytes(uint8_t *map, size_t length, long *waited_us) {
    const char replacement[] = "REMOTE_VERSION";
    struct timespec start, now;
    clock_gettime(CLOCK_MONOTONIC, &start);
    *waited_us = 0;
    for (;;) {
        if (memcmp((const void *)map, replacement, sizeof(replacement) - 1) == 0) {
            return 0;
        }
        clock_gettime(CLOCK_MONOTONIC, &now);
        *waited_us = elapsed_us(start, now);
        if (*waited_us > 5000000) {
            (void)length;
            return 1;
        }
        struct timespec pause = {.tv_sec = 0, .tv_nsec = 1000000};
        nanosleep(&pause, NULL);
    }
}

static int cmd_wait_remote_invalidate(const char *mapped_path, const char *ready_path, const char *output_path) {
    int fd = open(mapped_path, O_RDONLY);
    if (fd < 0) {
        return fail_errno("open wait mapped path");
    }
    uint8_t *map = mmap(NULL, 4096, PROT_READ, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) {
        close(fd);
        return fail_errno("mmap wait remote reader");
    }
    volatile uint8_t before = map[0];
    if (before != 'O') {
        munmap(map, 4096);
        close(fd);
        return fail_msg("unexpected wait initial mapped byte");
    }
    int ready = open(ready_path, O_CREAT | O_TRUNC | O_WRONLY, 0644);
    if (ready < 0) {
        munmap(map, 4096);
        close(fd);
        return fail_errno("create ready file");
    }
    if (write_full(ready, "ready\n", 6) != 0 || close(ready) != 0) {
        munmap(map, 4096);
        close(fd);
        return fail_errno("write ready file");
    }
    long waited_us = 0;
    if (wait_for_remote_bytes(map, 4096, &waited_us) != 0) {
        munmap(map, 4096);
        close(fd);
        return fail_msg("mapped page still shows stale bytes after external remote write");
    }
    if (munmap(map, 4096) != 0) {
        close(fd);
        return fail_errno("munmap wait invalidate");
    }
    close(fd);
    FILE *output = fopen(output_path, "w");
    if (output == NULL) {
        return fail_errno("open wait output");
    }
    fprintf(output, "{\"check\":\"remote_invalidate_mapped_page\",\"status\":\"passed\",\"waited_us\":%ld}\n", waited_us);
    if (fclose(output) != 0) {
        return fail_errno("close wait output");
    }
    printf("{\"check\":\"remote_invalidate_mapped_page\",\"status\":\"passed\",\"waited_us\":%ld}\n", waited_us);
    return 0;
}

static int cmd_truncate_sigbus(const char *path) {
    if (create_pattern_file(path, 8192, 'S') != 0) {
        return fail_errno("create sigbus file");
    }
    int fd = open(path, O_RDWR);
    if (fd < 0) {
        return fail_errno("open sigbus");
    }
    uint8_t *map = mmap(NULL, 8192, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) {
        close(fd);
        return fail_errno("mmap sigbus");
    }
    volatile uint8_t first = map[4096];
    (void)first;
    if (ftruncate(fd, 4096) != 0) {
        munmap(map, 8192);
        close(fd);
        return fail_errno("truncate sigbus");
    }
    struct sigaction action;
    memset(&action, 0, sizeof(action));
    action.sa_handler = handle_sigbus;
    sigemptyset(&action.sa_mask);
    if (sigaction(SIGBUS, &action, NULL) != 0) {
        munmap(map, 8192);
        close(fd);
        return fail_errno("sigaction SIGBUS");
    }
    if (sigsetjmp(sigbus_jump, 1) == 0) {
        volatile uint8_t value = map[4096];
        (void)value;
    }
    munmap(map, 8192);
    close(fd);
    if (!saw_sigbus) {
        return fail_msg("access beyond truncated EOF did not raise SIGBUS");
    }
    printf("{\"check\":\"truncate_eof_sigbus\",\"status\":\"passed\"}\n");
    return 0;
}

static int cmd_punch_hole_zero(const char *path) {
    if (create_pattern_file(path, 8192, 'H') != 0) {
        return fail_errno("create punch file");
    }
    int fd = open(path, O_RDWR);
    if (fd < 0) {
        return fail_errno("open punch");
    }
    uint8_t *map = mmap(NULL, 8192, PROT_READ, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) {
        close(fd);
        return fail_errno("mmap punch");
    }
    volatile uint8_t before = map[0];
    if (before != 'H') {
        munmap(map, 8192);
        close(fd);
        return fail_msg("unexpected punch initial byte");
    }
    if (fallocate(fd, FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE, 0, 4096) != 0) {
        munmap(map, 8192);
        close(fd);
        return fail_errno("fallocate PUNCH_HOLE");
    }
    for (size_t index = 0; index < 4096; index++) {
        if (map[index] != 0) {
            munmap(map, 8192);
            close(fd);
            return fail_msg("punched mapped range did not read as zero");
        }
    }
    munmap(map, 8192);
    close(fd);
    printf("{\"check\":\"punch_hole_mapped_zero\",\"status\":\"passed\"}\n");
    return 0;
}

static int cmd_unlink_open_mmap(const char *path) {
    if (create_pattern_file(path, 4096, 'U') != 0) {
        return fail_errno("create unlink file");
    }
    int fd = open(path, O_RDWR);
    if (fd < 0) {
        return fail_errno("open unlink");
    }
    uint8_t *map = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (map == MAP_FAILED) {
        close(fd);
        return fail_errno("mmap unlink");
    }
    if (unlink(path) != 0) {
        munmap(map, 4096);
        close(fd);
        return fail_errno("unlink mapped file");
    }
    if (map[0] != 'U') {
        munmap(map, 4096);
        close(fd);
        return fail_msg("mapped bytes disappeared after unlink");
    }
    memcpy(map, "UNLINK_LIVE", 11);
    if (msync(map, 4096, MS_SYNC) != 0) {
        munmap(map, 4096);
        close(fd);
        return fail_errno("msync unlinked mapping");
    }
    munmap(map, 4096);
    close(fd);
    if (access(path, F_OK) == 0) {
        return fail_msg("unlinked path still exists");
    }
    printf("{\"check\":\"unlink_open_mmap_lifetime\",\"status\":\"passed\"}\n");
    return 0;
}

static int usage(const char *program) {
    fprintf(
        stderr,
        "usage: %s <shared-msync|private-no-publish|remote-invalidate|wait-remote-invalidate|truncate-sigbus|punch-hole-zero|unlink-open-mmap> PATH [PEER_PATH|READY OUTPUT]\n",
        program);
    return 2;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        return usage(argv[0]);
    }
    if (strcmp(argv[1], "shared-msync") == 0 && argc == 4) {
        return cmd_shared_msync(argv[2], argv[3]);
    }
    if (strcmp(argv[1], "private-no-publish") == 0 && argc == 3) {
        return cmd_private_no_publish(argv[2]);
    }
    if (strcmp(argv[1], "remote-invalidate") == 0 && argc == 4) {
        return cmd_remote_invalidate(argv[2], argv[3]);
    }
    if (strcmp(argv[1], "wait-remote-invalidate") == 0 && argc == 5) {
        return cmd_wait_remote_invalidate(argv[2], argv[3], argv[4]);
    }
    if (strcmp(argv[1], "truncate-sigbus") == 0 && argc == 3) {
        return cmd_truncate_sigbus(argv[2]);
    }
    if (strcmp(argv[1], "punch-hole-zero") == 0 && argc == 3) {
        return cmd_punch_hole_zero(argv[2]);
    }
    if (strcmp(argv[1], "unlink-open-mmap") == 0 && argc == 3) {
        return cmd_unlink_open_mmap(argv[2]);
    }
    return usage(argv[0]);
}
