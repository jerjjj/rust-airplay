/* Does a mirror packet read the keystream of the packet before it, or a block-aligned
 * part of the stream?
 *
 * The question needs no key derivation and no second implementation of the cipher: a
 * real reader reads the keystream of a packet when it is given zeros, so two real
 * readers — one given the first length, one given the first then the second — say what
 * the second packet read. The second answer is then looked for inside the first: if it
 * starts where the first packet's bytes end, the rule is adjacency; if it starts at the
 * next block boundary, the rule is alignment; and if neither, the offset that does
 * match is the rule.
 */

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "mirror_buffer.h"
#include "raop_rtp.h"

static const uint64_t connection_id = 2765102747429192913ULL;
static const unsigned char audio_key[16] = {
    0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a,
    0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a,
};

/* What a reader reads for one packet of `length` bytes, from a stream that starts at the
 * beginning. */
/* A client's packets are tens of kilobytes, so the buffers are sized for one. */
#define MAX_PACKET 131072

static void reads_of_one(unsigned char *model, int length) {
    static unsigned char input[MAX_PACKET];
    memset(input, 0, sizeof(input));
    memset(model, 0, MAX_PACKET);
    logger_t *logger = logger_init();
    mirror_buffer_t *reader = mirror_buffer_init(logger, audio_key);
    mirror_buffer_init_aes(reader, &connection_id);
    mirror_buffer_decrypt(reader, input, model, length);
    mirror_buffer_destroy(reader);
}

/* What a reader reads for the second of two packets, having read the first. */
static void reads_of_second(unsigned char *model, int first_length, int second_length) {
    static unsigned char input[MAX_PACKET];
    static unsigned char waste[MAX_PACKET];
    logger_t *logger = logger_init();
    mirror_buffer_t *reader = mirror_buffer_init(logger, audio_key);
    mirror_buffer_init_aes(reader, &connection_id);
    memset(input, 0, sizeof(input));
    memset(waste, 0, sizeof(waste));
    mirror_buffer_decrypt(reader, input, waste, first_length);
    memset(input, 0, sizeof(input));
    memset(model, 0, MAX_PACKET);
    mirror_buffer_decrypt(reader, input, model, second_length);
    mirror_buffer_destroy(reader);
}

static void report(int first_length, int second_length) {
    static unsigned char head[MAX_PACKET];
    static unsigned char second[MAX_PACKET];
    reads_of_one(head, first_length + second_length);
    reads_of_second(second, first_length, second_length);

    /* Where the second packet's answer sits in the stream the first packet shows.
     * Adjacency puts it at `first_length`; alignment puts it at the block boundary
     * after that. */
    int found = -1;
    for (int offset = 0; offset + second_length <= first_length + second_length; offset++) {
        if (memcmp(second, head + offset, (size_t) second_length) == 0) {
            found = offset;
            break;
        }
    }
    printf("packet of %d then %d: second reads at %d (adjacent would be %d, aligned %d)\n",
           first_length, second_length, found, first_length,
           ((first_length + 15) / 16) * 16);
}

int main(void) {
    /* Lengths from a real session: its first three video packets, and a stretch of its
     * smaller ones. A client's sizes are what this has to hold for. */
    report(62415, 1265);
    report(1265, 9710);
    report(9710, 1237);
    report(1356, 1355);
    report(1355, 1346);
    report(1237, 1236);
    report(33, 47);
    report(1, 1);
    return 0;
}
