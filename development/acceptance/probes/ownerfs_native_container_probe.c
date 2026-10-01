#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/statfs.h>
#include <unistd.h>

/* Experimental OCI Agent: verifies objects and host-data isolation before any
 * timings. It is deliberately not an adversarial container security audit. */
static void require(int condition, const char *what) {
    if (!condition) { fprintf(stderr, "container probe failed: %s (errno=%d)\n", what, errno); exit(2); }
}
static void quoted(const char *text) {
    putchar('"');
    for (const unsigned char *p = (const unsigned char *)text; *p; ++p) {
        if (*p == '"' || *p == '\\') { putchar('\\'); putchar(*p); }
        else if (*p < 32) printf("\\u%04x", *p);
        else putchar(*p);
    }
    putchar('"');
}
static void file_text(const char *path) {
    char text[65536]; int fd = open(path, O_RDONLY); require(fd >= 0, path);
    ssize_t n = read(fd, text, sizeof(text) - 1); require(n >= 0, "read diagnostic");
    text[n] = 0; require(close(fd) == 0, "close diagnostic"); quoted(text);
}
static void inaccessible(const char *path, int comma) {
    errno = 0; int fd = open(path, O_RDONLY); int error = errno;
    if (fd >= 0) close(fd);
    require(fd < 0 && (error == ENOENT || error == EACCES || error == ENOTDIR), "host path is accessible");
    printf("%s{\"path\":", comma ? "," : ""); quoted(path); printf(",\"errno\":%d}", error);
}
static void terminate(int signal_number) { (void)signal_number; _exit(0); }
int main(int argc, char **argv) {
    if (argc == 2 && !strcmp(argv[1], "--idle")) {
        require(signal(SIGTERM, terminate) != SIG_ERR, "init signal handler");
        for (;;) pause();
    }
    require(argc == 8, "arguments");
    struct stat root, parent; struct statfs fs;
    require(stat("/ownerfs/agent1", &root) == 0, "workspace stat");
    require((uint64_t)root.st_dev == strtoull(argv[1], NULL, 10) &&
            (uint64_t)root.st_ino == strtoull(argv[2], NULL, 10), "wrong workspace source object");
    require(stat("/ownerfs", &parent) == 0 && statfs("/ownerfs/agent1", &fs) == 0, "mount stat");
    DIR *dir = opendir("/ownerfs"); require(dir != NULL, "parent enumeration");
    struct dirent *entry; int count = 0;
    while ((entry = readdir(dir))) {
        if (!strcmp(entry->d_name, ".") || !strcmp(entry->d_name, "..")) continue;
        require(!strcmp(entry->d_name, "agent1"), "unexpected parent entry"); ++count;
    }
    require(count == 1 && closedir(dir) == 0, "only workspace is exposed");
    errno = 0; int fd = open("/ownerfs/unexpected", O_CREAT | O_EXCL | O_WRONLY, 0600);
    require(fd < 0 && errno == EROFS, "container parent must be readonly");
    char data[128]; fd = open("/ownerfs/agent1/data", O_RDONLY);
    require(fd >= 0, "own data open"); ssize_t n = read(fd, data, sizeof(data));
    require(n == 15 && !memcmp(data, "original-A-data", 15) && close(fd) == 0, "own data content");
    fd = open("/ownerfs/agent1/container-own-marker", O_WRONLY | O_CREAT | O_TRUNC, 0600);
    require(fd >= 0 && write(fd, "container-owned-data", 20) == 20 && close(fd) == 0, "own write");
    printf("{\"workspace\":{\"device\":%llu,\"inode\":%llu},\"fstype_magic\":%llu,\"parent\":{\"device\":%llu,\"inode\":%llu,\"entries\":[\"agent1\"],\"write_errno\":%d},\"pid\":%d,\"host_paths\":[",
           (unsigned long long)root.st_dev, (unsigned long long)root.st_ino,
           (unsigned long long)fs.f_type, (unsigned long long)parent.st_dev,
           (unsigned long long)parent.st_ino, EROFS, getpid());
    inaccessible("/ownerfs/agent1/../agent2/outside-sibling-marker", 0);
    inaccessible("/ownerfs/agent1/../outside-parent-marker", 1);
    for (int i = 4; i <= 6; ++i) inaccessible(argv[i], 1);
    char path[8192];
    int length = snprintf(path, sizeof(path), "/proc/%s/root%s", argv[3], argv[4]);
    require(length > 0 && (size_t)length < sizeof(path), "host proc path"); inaccessible(path, 1);
    length = snprintf(path, sizeof(path), "/proc/1/root%s", argv[4]);
    require(length > 0 && (size_t)length < sizeof(path), "container proc path"); inaccessible(path, 1);
    inaccessible(argv[7], 1);
    printf("],\"symlink_paths\":[");
    for (int i = 4; i <= 6; ++i) {
        snprintf(path, sizeof(path), "/ownerfs/agent1/escape-%d", i);
        unlink(path); require(symlink(argv[i], path) == 0, "own symlink create");
        inaccessible(path, i != 4); require(unlink(path) == 0, "own symlink unlink");
    }
    printf("],\"mountinfo\":"); file_text("/proc/self/mountinfo");
    printf(",\"status\":"); file_text("/proc/self/status");
    printf("}\n"); return 0;
}
