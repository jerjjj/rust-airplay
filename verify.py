import sys
import struct
from Crypto.Cipher import AES

if len(sys.argv) < 3:
    print("Usage: python verify.py <key_hex> <iv_hex>")
    print("Example: python verify.py e27178... bda881...")
    sys.exit(1)

key_hex = sys.argv[1]
iv_hex = sys.argv[2]

key = bytes.fromhex(key_hex)
iv = bytes.fromhex(iv_hex)

with open("encrypted_frame.bin", "rb") as f:
    enc = f.read()

cipher = AES.new(key, AES.MODE_CTR, nonce=b"", initial_value=iv)
dec = cipher.decrypt(enc)

nalu_size = struct.unpack(">I", dec[:4])[0]
print(f"NAL unit size: {nalu_size}")
print(f"First 32 hex: {dec[:32].hex()}")

with open("decrypted_frame.bin", "wb") as f:
    f.write(dec)
print(f"Saved decrypted_frame.bin ({len(dec)} bytes)")
