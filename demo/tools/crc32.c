/*
 * crc32.c -- CRC-32 (IEEE 802.3) with a runtime-built table.
 *
 * The same polynomial Ethernet and PCIe use, so this is the reference a
 * hardware CRC block gets checked against. Building the table at
 * startup rather than pasting 256 magic constants keeps the polynomial
 * visible -- the one thing you actually need to get right.
 *
 *     cc -O2 -Wall -Wextra -o /tmp/crc32 crc32.c && /tmp/crc32 --selftest
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Reversed representation of 0x04C11DB7, which is what a
 * least-significant-bit-first implementation wants. */
#define CRC32_POLY_REVERSED 0xEDB88320u

static uint32_t crc_table[256];
static int crc_table_ready = 0;

static void crc32_build_table(void)
{
    for (uint32_t i = 0; i < 256; i++) {
        uint32_t remainder = i;
        for (int bit = 0; bit < 8; bit++) {
            if (remainder & 1u) {
                remainder = (remainder >> 1) ^ CRC32_POLY_REVERSED;
            } else {
                remainder >>= 1;
            }
        }
        crc_table[i] = remainder;
    }
    crc_table_ready = 1;
}

uint32_t crc32_update(uint32_t crc, const void *data, size_t len)
{
    const uint8_t *bytes = (const uint8_t *)data;

    if (!crc_table_ready) {
        crc32_build_table();
    }

    crc ^= 0xFFFFFFFFu;
    for (size_t i = 0; i < len; i++) {
        crc = (crc >> 8) ^ crc_table[(crc ^ bytes[i]) & 0xFFu];
    }
    return crc ^ 0xFFFFFFFFu;
}

uint32_t crc32(const void *data, size_t len)
{
    return crc32_update(0u, data, len);
}

static int selftest(void)
{
    struct {
        const char *input;
        uint32_t expected;
    } cases[] = {
        { "", 0x00000000u },
        { "a", 0xE8B7BE43u },
        { "abc", 0x352441C2u },
        { "123456789", 0xCBF43926u },
        { "The quick brown fox jumps over the lazy dog", 0x414FA339u },
    };

    int failures = 0;
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        uint32_t got = crc32(cases[i].input, strlen(cases[i].input));
        if (got != cases[i].expected) {
            fprintf(stderr, "FAIL \"%s\": got 0x%08X, want 0x%08X\n",
                    cases[i].input, got, cases[i].expected);
            failures++;
        }
    }

    /* Feeding the data in two chunks must match one call, otherwise the
     * streaming interface is broken in a way the fixed vectors above
     * would never catch. */
    const char *split = "123456789";
    uint32_t streamed = crc32_update(0u, split, 4);
    streamed = crc32_update(streamed, split + 4, 5);
    if (streamed != 0xCBF43926u) {
        fprintf(stderr, "FAIL streaming: got 0x%08X, want 0xCBF43926\n",
                streamed);
        failures++;
    }

    if (failures == 0) {
        printf("PASS: %zu vectors + streaming\n",
               sizeof(cases) / sizeof(cases[0]));
    }
    return failures == 0 ? 0 : 1;
}

int main(int argc, char **argv)
{
    if (argc == 2 && strcmp(argv[1], "--selftest") == 0) {
        return selftest();
    }

    if (argc < 2) {
        fprintf(stderr, "usage: %s --selftest | %s STRING...\n", argv[0], argv[0]);
        return 2;
    }

    for (int i = 1; i < argc; i++) {
        printf("0x%08X  %s\n", crc32(argv[i], strlen(argv[i])), argv[i]);
    }
    return 0;
}
