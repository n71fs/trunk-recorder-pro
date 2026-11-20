# DMR and Hytera GPS positions

The recorder decodes GPS Info full link control embedded in voice calls (FLCO
`0x08`), for standard DMR (FID `0x00`) and Hytera (FID `0x68`), including XPT.
No configuration is required. Positions use the same `TrackerOut::Position`,
call JSON `srcList` latitude/longitude fields, and `Event::UnitPosition` path as
P25. Decoder source names are `DMR` and `HyteraDMR`.

The existing embedded-LC FEC and checksum handling, including keyed-system
handling, runs first. A position belongs to the current slot's talker, named
by voice LC less than four seconds earlier. A voice terminator (including
Hytera FLCO `0x30`) clears it. Protected words, unknown/zero talkers, and `(0, 0)`
are ignored. A fix on only the equator or prime meridian is accepted. Accuracy
`7` means unknown accuracy; it does not alone invalidate a nonzero position.
Positions are read even when the call's audio is encrypted.

Standalone packet-data services (Hytera HDAP/LP, Motorola LRRP, UDT/NMEA) are
outside this embedded voice GPS implementation.

## Wire format and sources

Ranges below exclude their end; bit 0 is the first byte's MSB. Coordinates
use two's complement, not sign/magnitude; north/east are positive.

| Bits | Meaning |
| --- | --- |
| 0..8 | Protect/reserved flags and six-bit FLCO `0x08` |
| 8..16 | Standard or Hytera FID |
| 16..20 | Reserved |
| 20..23 | Position error / accuracy |
| 23..48 | Signed longitude × 360 / 2²⁵ degrees |
| 48..72 | Signed latitude × 180 / 2²⁴ degrees |

Sources researched for the original implementation:

- [ETSI TS 102 361-2 V2.4.1](https://www.etsi.org/deliver/etsi_ts/102300_102399/10236102/02.04.01_60/ts_10236102v020401p.pdf),
  §7.1.1.3 / Table 7.3 and §§7.2.15–17: normative standard DMR layout and coordinate units.
- SDRTrunk [LCOpcode](https://github.com/DSheirer/sdrtrunk/blob/master/src/main/java/io/github/dsheirer/module/decode/dmr/message/data/lc/LCOpcode.java),
  [LCMessageFactory](https://github.com/DSheirer/sdrtrunk/blob/master/src/main/java/io/github/dsheirer/module/decode/dmr/message/data/lc/LCMessageFactory.java),
  and [GPSInformation](https://github.com/DSheirer/sdrtrunk/blob/master/src/main/java/io/github/dsheirer/module/decode/dmr/message/data/lc/full/GPSInformation.java):
  evidence that Hytera uses the same layout with FID `0x68`; also identifies
  Hytera's terminator opcode. GPSInformation's example is a regression vector.
- SDRTrunk [issue #1907](https://github.com/DSheirer/sdrtrunk/issues/1907):
  Hytera XPT hemisphere errors, covered here by signed-coordinate tests.

The Hytera interpretation is supported by implementation evidence; ETSI does
not specify that vendor extension.

## Release compatibility

Compatibility work on 2026-11-20 targets published
[v0.3.9](https://github.com/TrunkRecorder/trunk-recorder-pro/releases/tag/v0.3.9),
commit `5968e907959d64bbb17c5f7b37e9852ff3545cdb`.
The release normalizes XPT voice IDs in `Channel`, assembles talker aliases in
`DmrVoice`, and vocodes both slots without the old `vocode` switch. GPS keeps
those behaviors and shares the existing voice-LC timestamp for attribution.

Tests cover standard/Hytera coordinates in all hemispheres, no fix and protected
words, slot isolation, transmission end, stale talkers and refresh, and synthetic
4FSK through embedded-LC FEC. Engine tests verify position events and saved call
JSON for both conventional and trunked DMR; XPT tests verify normalized radio
IDs without truncating coordinate bits. Release-specific tests interleave GPS
and alias blocks to verify both outputs survive.
