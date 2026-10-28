# AirPlay screen mirroring: protocol notes

What Riflesso X implements, as observed with iOS 26 (AirPlay 980.77). These notes
complement the code in `crates/airplay`; the open-source receivers
[UxPlay](https://github.com/FDH2/UxPlay) and [RPiPlay](https://github.com/FD-/RPiPlay)
were the protocol references.

## Discovery

Two Bonjour services on port 7000, `_airplay._tcp` and `_raop._tcp`
(`crates/airplay/src/mdns.rs`), advertised on IPv4 only. Hostname queries for IPv6
(`AAAA`) are answered with an NSEC record ("no such address", RFC 6762 §6.1), as
Bonjour does.

The receiver presents itself as an `AppleTV3,2` running `srcvers` 220.68:

| Feature bits | Meaning |
|---|---|
| `0x527FFEE6` | video, photo, mirroring, audio, FairPlay (12, 14), RAOP (30) |
| `+ 1 << 42` (`0x527FFEE6,0x400`) | ScreenMultiCodec: H.265 allowed |

**Bit 27 (legacy pairing) stays off.** With it, iPhones wait 4–6 seconds on
"Connecting…" before opening any connection, while the whole handshake up to the
first picture takes 150–250 ms. Without it the sender skips `pair-setup` and
`pair-verify`.

The sender picks the stream size from the **height** of the advertised display: with
H.264 at most 1080 lines (498×1080 in portrait); above 1080 it uses H.265.

`GET /info` returns a binary plist. A request carrying a `qualifier`
(`txtAirPlay` / `txtRAOP`) gets the raw TXT record instead.

## Handshake

1. `pair-setup` / `pair-verify` (only with bit 27): Ed25519 + X25519. Step 1 returns
   our ephemeral key and our signature of (ours ‖ theirs), encrypted with AES-128-CTR,
   key/IV = SHA-512("Pair-Verify-AES-Key" / "Pair-Verify-AES-IV" ‖ shared secret)[:16].
   The client's signature in step 2 continues the same keystream (skip 64 bytes).
2. `fp-setup`, 16 bytes: one of four fixed 142-byte replies, chosen by `request[14]`.
   164 bytes: reply = `FPLY` header (12 bytes) + `request[144..164]`.
3. First `SETUP`, with `ekey` (72 bytes) and `eiv`: the session key is
   `playfair_decrypt(fp-setup message, ekey)`; after pair-verify it becomes
   SHA-512(key ‖ ECDH secret)[:16].

## Video (stream type 110)

- Key and IV: SHA-512("AirPlayStreamKey{id}" ‖ key)[:16] and
  SHA-512("AirPlayStreamIV{id}" ‖ key)[:16], where `id` is `streamConnectionID`.
  One **continuous AES-128-CTR** keystream covers the whole stream.
- Packets: 128-byte header (`[0..4]` size LE, `[4]` type, `[8..16]` timestamp) + payload.
  Type 0: encrypted frame (NAL units with 4-byte big-endian lengths). Type 1: codec
  configuration in clear (`avcC`, or an `hvc1` sample entry containing `hvcC`), with
  sizes as LE floats at `[40]`, `[44]` (source) and `[56]`, `[60]`. Types 2 and 5 are
  ignored.
- **Colours are full range (0–255), BT.709** (VUI `full_range=1`, matrix 1, transfer 13).
  Treating them as limited range gives a warm, washed tint.

## Audio (stream type 96)

- `ct` 8 = AAC-ELD (mirroring), 4 = AAC-LC, 2 = ALAC (not supported yet).
  ELD AudioSpecificConfig: `F8 E8 50 00` (44.1 kHz stereo, 480-sample frames, no SBR).
- RTP with a 12-byte header (sequence number BE at `[2..4]`). Payload in **AES-128-CBC**
  with the session key and `eiv`, restarting from the IV for every packet; only whole
  16-byte blocks are encrypted, the tail is in clear.
- iOS sends **every packet three times**: drop duplicates by sequence number. Early
  packets with the marker `00 68 34 00` carry no audio.
- Control port: `0xD6` = retransmission (data packet from byte 4), `0xD4` = sync.
- Volume: `SET_PARAMETER` with `volume: <dB>` (−30..0, −144 = muted).

## Access control

All modes use **HTTP Digest** (RFC 2617, MD5) on the first `SETUP`
(`crates/airplay/src/auth.rs`, `session.rs::authorize`). The TXT records then carry
`pw=true` and `sf=0x84`.

- The challenge is `401` with `WWW-Authenticate: Digest realm="raop", nonce="<32 hex>"`.
  Each nonce serves a single attempt and is shared by all connections, because the
  sender retries on a new one after prompting.
- The sender shows a **password** prompt. Right after a `401` it silently retries with
  the password it remembers for this receiver.
- *Password*: a fixed password. *Code*: a random 4-digit code shown on the PC is the
  password; devices that typed it are remembered (`data/paired_devices.txt`) and their
  remembered code is accepted silently. *Code every time*: a new code each time.

Apple's own on-screen code, **pair-setup-pin**, was implemented and then removed,
because it needs bit 27 and therefore the 4–6 second wait. For reference:
`POST /pair-pin-start` (show a code), then three `POST /pair-setup-pin` exchanges in
binary plists: SRP-6a with the RFC 5054 2048-bit group, g = 2, SHA-1, Apple's 40-byte
session key K = SHA1(S ‖ 00000000) ‖ SHA1(S ‖ 00000001), then the client's long-term
Ed25519 key and ours exchanged with AES-128-GCM (16-byte IV from
SHA-512("Pair-Setup-AES-IV" ‖ K), last byte +1 and +2). Unknown clients get `470` at
`pair-verify` and iOS falls back to `pair-pin-start` by itself.

## Windows specifics

- Media Foundation's AAC decoder cannot decode AAC-ELD, so audio uses `rusty_aac`.
- The HEVC decoder only offers output types if the input type has `MF_MT_FRAME_SIZE`
  (any size; a format change follows).
- The D3D11 video processor rejects NV12 textures created with
  `BIND_SHADER_RESOURCE`; the software-upload texture uses no bind flags.
- Many audio devices only accept 48 kHz: audio is resampled, also to follow the clock
  drift between the sender and the sound card.
