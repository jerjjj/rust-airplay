/* What OpenSSL's AES-CTR produces, block by block, for one long buffer.
 *
 * The mirror rule is "which keystream block does a byte come from", so if a reader's
 * counter moves by more than one block somewhere in the middle of a long packet, that is
 * visible here without any AirPlay code involved at all: encrypt a long buffer of zeros and
 * compare every block with the block the previous one should be followed by.
 *
 * Two ways of doing it are printed, because they can differ: one call for the whole buffer,
 * and one call per block. OpenSSL's counter carries between the low 32 bits and the rest of
 * the 128-bit counter, and a call boundary is where a carry can be lost.
 */

#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "crypto.h"

/* The key and IV the mirror stream uses. They are derived from an audio key and a
 * connection identifier, and the derivation is upstream's; what is under test here is the
 * cipher, so the same values are used for both runs and the second one is a fresh context
 * over the same bytes. */
static const unsigned char key[16] = {
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
    0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
};
static const unsigned char iv[16] = {
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
    0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0xfe, 0xff,
};

#define BLOCKS 4096

static unsigned char whole[BLOCKS * 16];
static unsigned char per_block[BLOCKS * 16];

int main(void) {
    /* One call for the whole buffer. */
    aes_ctx_t *all = aes_ctr_init(key, iv);
    unsigned char zeros[BLOCKS * 16];
    memset(zeros, 0, sizeof(zeros));
    aes_ctr_decrypt(all, zeros, whole, (int)sizeof(zeros));
    aes_ctr_destroy(all);

    /* One call per block, which is where a carry between the low 32 bits and the rest of
     * the counter would be lost. */
    aes_ctx_t *each = aes_ctr_init(key, iv);
    for (int block = 0; block < BLOCKS; block++) {
        memset(zeros, 0, 16);
        aes_ctr_decrypt(each, zeros, per_block + block * 16, 16);
    }
    aes_ctr_destroy(each);

    int differing = 0;
    for (int block = 0; block < BLOCKS; block++) {
        if (memcmp(whole + block * 16, per_block + block * 16, 16) != 0) {
            differing++;
            if (differing <= 8) {
                printf("block %d differs between one call and one call per block\n", block);
            }
        }
    }
    printf("%d of %d blocks differ\n", differing, BLOCKS);

    /* The counter's own path, read out of the cipher: the block after this one produces is
     * the counter plus the blocks consumed, so a counter that gains 256 somewhere shows up
     * as a block whose bytes are the ones 256 blocks on. */
    aes_ctx_t *walk = aes_ctr_init(key, iv);
    unsigned char block_bytes[16];
    unsigned char ahead[16];
    for (int block = 0; block < 600; block++) {
        memset(zeros, 0, 16);
        aes_ctr_decrypt(walk, zeros, block_bytes, 16);
        if (block >= 170 && block <= 186) {
            printf("block %3d: %02x%02x%02x%02x...\n", block, block_bytes[0], block_bytes[1],
                   block_bytes[2], block_bytes[3]);
        }
    }
    aes_ctr_destroy(walk);
    (void)ahead;
    return 0;
}
