#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int fstat_with_fault(int fd, struct stat *stat_buffer) {
    const char *target = getenv("IRONRDP_TEST_FSTAT_PATH");
    const char *marker = getenv("IRONRDP_TEST_FSTAT_MARKER");
    char actual[PATH_MAX];
    if (target != NULL && marker != NULL && fcntl(fd, F_GETPATH, actual) == 0 &&
        strcmp(target, actual) == 0 && access(marker, F_OK) == 0) {
        int marker_fd = open(marker, O_WRONLY | O_APPEND);
        if (marker_fd != -1) {
            const char event = 'x';
            (void)write(marker_fd, &event, 1);
            (void)close(marker_fd);
        }
        errno = EIO;
        return -1;
    }
    return fstat(fd, stat_buffer);
}

__attribute__((used)) static const struct {
    const void *replacement;
    const void *original;
} fstat_interpose __attribute__((section("__DATA,__interpose"))) = {
    (const void *)fstat_with_fault,
    (const void *)fstat,
};
