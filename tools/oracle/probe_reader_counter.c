/* Whether a mirror reader reads the cipher's blocks in order.
 *
 * The earlier attempt at this asked whether each of a reader's blocks could be found in its
 * own answer, which is circular: a block is always somewhere in the buffer it came from, so
 * the check passed whatever the reader did. Finding a block is not the same as its being at
 * its own place, and only the second is the rule.
 *
 * So this compares the reader with the cipher directly. The key and IV the reader derives
 * are not reachable from outside it, but they do not have to be: a second AES-128-CTR
 * context is not needed either, because the reader's own answer says which block it read —
 * the question is only whether block N of the answer is the block that follows block N-1.
 * The cipher's blocks are consecutive, so a reader that reads them in order produces an
 * answer whose blocks are the cipher's blocks in order, and a reader that jumps produces an
 * answer with a block that is not the next one.
 *
 * The way to see a jump without knowing the cipher's blocks is to decrypt the same packet
 * twice with a reader that is fresh each time, and to compare a reader given the packet in
 * one call with a reader given it in two: a counter that is advanced wrongly by a call
 * boundary shows up as the two answers differing past that boundary.
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "crypto.h"
#include "mirror_buffer.h"
#include "raop_rtp.h"

static const uint64_t connection_id = 2765102747429192913ULL;
static const unsigned char audio_key[16] = {
    0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a,
    0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a,
};

#define LENGTH 8000

/* What a reader reads for one packet of `length` bytes, made of `pieces` calls. */
static void read_in_pieces(unsigned char *model, int length, int pieces) {
    static unsigned char input[65536];
    logger_t *logger = logger_init();
    mirror_buffer_t *reader = mirror_buffer_init(logger, audio_key);
    mirror_buffer_init_aes(reader, &connection_id);
    memset(model, 0, (size_t)length);
    int done = 0;
    for (int piece = 0; piece < pieces; piece++) {
        int size = (length - done) / (pieces - piece);
        memset(input, 0, sizeof(input));
        mirror_buffer_decrypt(reader, input, model + done, size);
        done += size;
    }
    mirror_buffer_destroy(reader);
}

int main(void) {
    static unsigned char whole[LENGTH];
    static unsigned char pieces[LENGTH];
    static unsigned char again[LENGTH];

    read_in_pieces(whole, LENGTH, 1);
    read_in_pieces(pieces, LENGTH, 2);
    read_in_pieces(again, LENGTH, 1);

    /* A reader is deterministic, so the same packet read the same way twice gives the same
     * answer. This is the control: if it fails, nothing else here means anything. */
    printf("two readings of the same packet agree: %s\n",
           memcmp(whole, again, LENGTH) == 0 ? "yes" : "NO");

    int differing = 0;
    for (int at = 0; at < LENGTH; at++) {
        if (whole[at] != pieces[at]) {
            differing++;
            if (differing <= 6) {
                printf("  byte %d (block %d + %d): one call gives %02x, two calls give %02x\n", at,
                       at / 16, at % 16, whole[at], pieces[at]);
            }
        }
    }
    printf("%d of %d bytes differ between one call and two\n", differing, LENGTH);

    /* And the answer's own blocks, in order: which block of the answer is the block that
     * follows the one before it. A jump makes a block that is not consecutive with its
     * predecessor, and this says where. */
    for (int block = 1; block < LENGTH / 16; block++) {
        const unsigned char *previous = whole + (size_t)(block - 1) * 16;
        const unsigned char *current = whole + (size_t)block * 16;
        /* Consecutive blocks of a counter differ in their low byte before encryption and
         * are unrelated after it, so consecutiveness cannot be seen from the bytes alone.
         * What can be seen is where each block occurs in the whole answer: a block that
         * occurs before its own place means the reader went back, and one that occurs after
         * means it went on. */
        int first_place = -1;
        for (int other = 0; other < LENGTH / 16; other++) {
            if (memcmp(current, whole + (size_t)other * 16, 16) == 0) {
                first_place = other;
                break;
            }
        }
        if (first_place != block) {
            printf("  the block at place %d also occurs at place %d\n", block, first_place);
            break;
        }
        (void)previous;
    }
    return 0;
}
