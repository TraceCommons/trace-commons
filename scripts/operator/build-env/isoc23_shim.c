// Shim: bridge ort precompiled binary built against glibc 2.38+ to
// host glibc 2.35. The __isoc23_* variants differ from their non-C23
// counterparts only in stricter overflow behavior on edge cases; for
// ORT inference paths this is benign.
#include <stdlib.h>
#include <locale.h>

long __isoc23_strtol(const char *nptr, char **endptr, int base) {
    return strtol(nptr, endptr, base);
}
long long __isoc23_strtoll(const char *nptr, char **endptr, int base) {
    return strtoll(nptr, endptr, base);
}
long long __isoc23_strtoll_l(const char *nptr, char **endptr, int base, locale_t loc) {
    return strtoll_l(nptr, endptr, base, loc);
}
unsigned long __isoc23_strtoul(const char *nptr, char **endptr, int base) {
    return strtoul(nptr, endptr, base);
}
unsigned long long __isoc23_strtoull(const char *nptr, char **endptr, int base) {
    return strtoull(nptr, endptr, base);
}
unsigned long long __isoc23_strtoull_l(const char *nptr, char **endptr, int base, locale_t loc) {
    return strtoull_l(nptr, endptr, base, loc);
}
