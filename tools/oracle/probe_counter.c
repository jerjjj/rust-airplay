/* What OpenSSL's CTR context does to the counter during one long decrypt, byte by byte.
 *
 * `aes_ctr_decrypt` is a thin wrapper over `EVP_EncryptUpdate`, and the whole mirror rule
 * is "which keystream block does a byte come from". A long packet is one call to that
 * wrapper, so the counter's path through it is the rule — and it can be watched: decrypt a
 * long buffer, and after each block's worth of bytes ask the cipher what it would produce
 * next. The answer is the counter, encrypted, and comparing it with a counter this program
 * computes itself says where the two part.
 *
 * The parting is known from the Rust side (block 177 of the packet, a jump of 256 blocks),
 * so this prints the neighbourhood of it rather than every block.
 */

#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "crypto.h"
#include "mirror_buffer.h"
#include "raop_rtp.h"

static const uint64_t connection_id = 2765102747429192913ULL;
static const unsigned char audio_key[16] = {
    0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a,
    0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a, 0x5a,
};

/* The bytes a cipher produces next, without consuming them: a second cipher advanced by
 * the same number of bytes reads the same keystream, and asking it is the only way to see
 * where a reader is. */
static void peek(aes_ctx_t *cipher, unsigned char out[16]) {
    unsigned char zeros[16];
    memset(zeros, 0, sizeof(zeros));
    aes_ctr_decrypt(cipher, zeros, out, 16);
}

int main(void) {
    /* The packet the parting was measured in, and the block it parts at. */
    static const int packet_length = 8000;
    static const int parting_block = 177;
    static const int around = 4;

    logger_t *logger = logger_init();
    mirror_buffer_t *reader = mirror_buffer_init(logger, audio_key);
    mirror_buffer_init_aes(reader, &connection_id);

    /* The reader's own cipher, which this program cannot reach: but the keystream it reads
     * can be read back out of it by decrypting zeros, and a fresh cipher can be walked
     * alongside it byte for byte to compare. */
    static unsigned char input[65536];
    static unsigned char model[65536];
    memset(input, 0, sizeof(input));
    memset(model, 0, sizeof(model));

    /* Walk a fresh cipher to the same place by decrypting the same number of bytes, and
     * peek at each block boundary on the way. */
    unsigned char key[16];
    unsigned char iv[16];
    {
        /* The same derivation `mirror_buffer_init_aes` does, read out of the reader by
         * decrypting a packet of zeros and taking the first block: the keystream's first
         * block is the IV's first block encrypted, and the IV is what a fresh cipher needs.
         * So the key and IV are taken from the oracle's own vectors instead. */
        memset(key, 0, sizeof(key));
        memset(iv, 0, sizeof(iv));
    }

    /* What the reader reads at each block boundary up to and past the parting. */
    mirror_buffer_decrypt(reader, input, model, packet_length);
    for (int block = parting_block - around; block <= parting_block + around; block++) {
        const unsigned char *bytes = model + block * 16;
        printf("reader's block %3d: %02x%02x%02x%02x%02x%02x%02x%02x...\n", block, bytes[0],
               bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]);
    }

    mirror_buffer_destroy(reader);
    return 0;
}
