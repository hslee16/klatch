"""Computes the klatch format vectors pinned in crypto.rs without using any klatch code (#624).

It implements XChaCha20-Poly1305 from scratch and first checks itself against the published vector (draft-arciszewski-xchacha-03, appendix A.1), so the values it prints do not come from the code under test. Run: python3 -I crypto-vectors.py. Standard library only. Not for real use: it is slow and not constant time.
"""
import struct, hashlib, base64

M = 0xFFFFFFFF
def rotl(v, n): return ((v << n) & M) | (v >> (32 - n))
def qr(s, a, b, c, d):
    s[a] = (s[a] + s[b]) & M; s[d] = rotl(s[d] ^ s[a], 16)
    s[c] = (s[c] + s[d]) & M; s[b] = rotl(s[b] ^ s[c], 12)
    s[a] = (s[a] + s[b]) & M; s[d] = rotl(s[d] ^ s[a], 8)
    s[c] = (s[c] + s[d]) & M; s[b] = rotl(s[b] ^ s[c], 7)
def rounds(s):
    for _ in range(10):
        qr(s,0,4,8,12); qr(s,1,5,9,13); qr(s,2,6,10,14); qr(s,3,7,11,15)
        qr(s,0,5,10,15); qr(s,1,6,11,12); qr(s,2,7,8,13); qr(s,3,4,9,14)
CONST = [0x61707865, 0x3320646e, 0x79622d32, 0x6b206574]
def block(key, counter, nonce12):
    s = CONST + list(struct.unpack('<8I', key)) + [counter] + list(struct.unpack('<3I', nonce12))
    w = s[:]
    rounds(w)
    return struct.pack('<16I', *[(a + b) & M for a, b in zip(w, s)])
def hchacha(key, n16):
    s = CONST + list(struct.unpack('<8I', key)) + list(struct.unpack('<4I', n16))
    rounds(s)
    return struct.pack('<8I', *(s[0:4] + s[12:16]))
def stream_xor(key, nonce12, data, counter=1):
    out = bytearray()
    for i in range(0, len(data), 64):
        ks = block(key, counter + i // 64, nonce12)
        out += bytes(a ^ b for a, b in zip(data[i:i+64], ks))
    return bytes(out)
def poly1305(key, msg):
    r = int.from_bytes(key[:16], 'little') & 0x0ffffffc0ffffffc0ffffffc0fffffff
    s = int.from_bytes(key[16:], 'little')
    p = (1 << 130) - 5
    a = 0
    for i in range(0, len(msg), 16):
        n = int.from_bytes(msg[i:i+16] + b'\x01', 'little')
        a = ((a + n) * r) % p
    return ((a + s) & ((1 << 128) - 1)).to_bytes(16, 'little')
def pad16(b): return b + b'\x00' * (-len(b) % 16)
def xchacha_seal(key, nonce24, aad, pt):
    sub = hchacha(key, nonce24[:16])
    n12 = b'\x00' * 4 + nonce24[16:]
    otk = block(sub, 0, n12)[:32]
    ct = stream_xor(sub, n12, pt)
    mac = poly1305(otk, pad16(aad) + pad16(ct) + struct.pack('<QQ', len(aad), len(ct)))
    return ct + mac

if __name__ == '__main__':
    # check against the official vector first
    key = bytes(range(0x80, 0xa0)); aad = bytes.fromhex('50515253c0c1c2c3c4c5c6c7')
    pt = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it."
    n = bytes.fromhex('404142434445464748494a4b4c4d4e4f5051525354555657')
    got = xchacha_seal(key, n, aad, pt).hex()
    want = ('bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b4522f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff921f9664c97637da9768812f615c68b13b52e'
            'c0875924c1c7987947deafd8780acf49')
    assert got == want, 'oracle does not match the official vector'
    print('oracle matches the official vector')
    # klatch sealed value
    k = bytes(range(32)); nonce = bytes(range(0xa0, 0xb8)); ctx = b'message:lobby'; plain = b'the code is hunter2'
    blob = b'KS' + bytes([1, 1]) + nonce + xchacha_seal(k, nonce, ctx, plain)
    print('BLOB', blob.hex())
    print('TEXT', 'klatch-sealed:1:' + base64.urlsafe_b64encode(blob).decode().rstrip('='))
    # recovery code
    chk = hashlib.sha256(b'klatch recovery v1' + k).digest()[:2]
    b32 = base64.b32encode(k + chk).decode().rstrip('=')
    print('CODE', 'KLATCH-' + '-'.join(b32[i:i+4] for i in range(0, len(b32), 4)))
    # token hash
    print('TOKHASH', hashlib.sha256(b'klatch credential v1' + k).hexdigest())
    import base64 as b
    print('TOKTEXT', 'KLATCH-CRED-' + b.b32encode(k).decode().rstrip('='))
