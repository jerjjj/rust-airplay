#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>

extern void playfair_decrypt(unsigned char* message3, unsigned char* cipherText, unsigned char* keyOut);

int main(int argc, char** argv) {
    if (argc < 3) { printf("Usage: %s key_msg.bin ekey.bin\n", argv[0]); return 1; }
    
    FILE* f = fopen(argv[1], "rb");
    fseek(f, 0, SEEK_END); long klen = ftell(f); fseek(f, 0, SEEK_SET);
    unsigned char* key_msg = malloc(klen); fread(key_msg, 1, klen, f); fclose(f);
    
    f = fopen(argv[2], "rb");
    fseek(f, 0, SEEK_END); long elen = ftell(f); fseek(f, 0, SEEK_SET);
    unsigned char* ekey = malloc(elen); fread(ekey, 1, elen, f); fclose(f);
    
    printf("key_msg: %ld bytes\n", klen);
    printf("ekey:    %ld bytes\n", elen);
    
    unsigned char aes_key[16];
    playfair_decrypt(key_msg, ekey, aes_key);
    
    printf("AES key: ");
    for (int i = 0; i < 16; i++) printf("%02x", aes_key[i]);
    printf("\n");
    
    free(key_msg); free(ekey);
    return 0;
}
