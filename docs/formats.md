# File formats

Layouts as MantaRay implements them, and how each was established.

## `.Chn` - ORTEC integer spectrum

Little endian throughout. The layout comes from the reader published in Appendix B
of the MAESTRO manual; the century flag and the 20 ms tick were confirmed by
round-tripping real files.

```text
offset size field
0      2    i16   file type, always -1
2      2    i16   MCA (detector) number
4      2    i16   segment number
6      2    ascii seconds of the start time
8      4    i32   real time, in 20 ms ticks
12     4    i32   live time, in 20 ms ticks
16     8    ascii start date "DDMMMYY" plus a century flag ('1' = 2000s, ' ' = 1900s)
24     4    ascii start time "HHMM"
28     2    u16   channel offset
30     2    u16   number of channels
32     4*n  i32   channel data
```

Then a 512-byte trailer:

```text
0      2    i16   -102 with a quadratic term, -101 without
2      2    i16   reserved
4      12   3 f32 energy calibration: zero, gain, quadratic
16     12   3 f32 peak-shape calibration: zero, linear, quadratic (in channels)
28     228  reserved
256    64   1 + 63 detector description: length byte, then text
320    64   1 + 63 sample description: length byte, then text
384    128  reserved
```

The trailer is optional on read: files that stop after the channel data load
without a calibration. Counts above `i32::MAX` are clamped. Regions are not part
of the format - save them to a `.Roi` file.

## `.Spc` - ORTEC binary spectrum

128-byte records; record `n` starts at byte `(n - 1) * 128`. Record 1 holds
pointers and scalars, and the reader follows them rather than assuming a fixed
record map, so any channel count works.

```text
record 1, byte offsets
0    u16  INFTYP  information type, 1 for a spectrum
2    u16  FILTYP  1 = integer channel data, 2 = floating point
8    u16  ACQIRP  record holding the acquisition information
10   u16  SAMDRP  record holding the sample description
12   u16  DETDRP  record holding the detector description
34   u16  CALRP1  first calibration record
36   u16  CALRP2  second calibration record
40   u16  ROIRP1  first region record
54   u16  MAXRCS  records in the file
56   u16  LSTREC  last record used; holds the energy calibration
60   u16  SPCTRP  first record of the channel data
62   u16  SPCRCN  number of channel-data records, 32 channels each
64   u16  SPCCHN  number of channels
66   u16  ABSTCH  absolute start channel
68   f32  ACQTIM  acquisition time
72   f64  ACQTI8  acquisition time, double precision
82   u16  MCANU   detector number
84   u16  SEGNUM  segment number
88   u16  CHNSRT  first channel stored
90   f32  RLTMDT  real time in seconds
94   f32  LVTMDT  live time in seconds

acquisition information record
0    16 ascii  default file name
16   12 ascii  date, "DD-MMM-YY" plus a century flag
28   10 ascii  time, "HH:MM:SS"
38   10 ascii  live time in seconds
48   10 ascii  real time in seconds
92   10 ascii  collection start date
102   8 ascii  collection start time
110  10 ascii  collection stop date
120   8 ascii  collection stop time

last record: three f32 energy calibration coefficients
```

**How this was verified.** The record map is documented in ORTEC's *Software File
Structure Manual*, which is not part of the MAESTRO user manual. The offsets used
here were checked against a real `TransSpec` file that also exists as a `.Spe`
conversion: both give 8192 channels, a live time of 900 s, a real time of 905.42 s,
a start of 2012-09-17 13:41:07 (from `17-SEP-12` plus century flag `1`), a
calibration of `0.5783 + 0.374436·ch + 2.985859e-7·ch²`, the sample description
`Alcatraz14` and the detector description `Transpec MCB129`. The layout table in
LBNL's [becquerel](https://github.com/lbl-anp/becquerel) parser was a useful
cross-check. `tests/spc_cross_check.rs` re-runs this comparison for any
`X.Spc`/`X.spe` pair placed in the fixtures directory.

**Not decoded.** The region records (`ROIRP1`) - their layout is not in the
material this was written from, and writing guesses into a file is worse than
leaving it out. Save regions to `.Roi`.

## `.Spe` - IAEA / CTBTO ASCII spectrum

`$KEYWORD:` blocks, each followed by its lines. Read and written:
`$SPEC_ID`, `$SPEC_REM` (with `DET#`, `DETDESC#`, `AP#`), `$DATE_MEA`,
`$MEAS_TIM` (live then real), `$DATA` (first and last channel, then one count per
line), `$ROI`, `$PRESETS`, `$ENER_FIT`, `$MCA_CAL`, `$SHAPE_CAL`. Unknown keywords
are ignored, so files carrying `$LATITUDE`, `$ELEVATION` or `$ENDRECORD` load
without complaint. CRLF, blank lines, fractional seconds in `$DATE_MEA` and a
missing units token in `$MCA_CAL` are all tolerated, because real files contain
all of them. Channels missing from the end of `$DATA` are zero filled.

## `.Roi` - region table

16-bit words: two header words, then `first` and `last + 1` for each region,
ending at a word that is zero or negative. A region touching channel 0 is stored
from channel 1, because zero terminates the list.

## ASCII dumps

TRANSLT's text format:

```text
Real Time   240
Live Time   120
     0:      10      12      13      11       9
     5:      14      16      12      10       8
```

`--columns`, `--no-channels` and `--no-header` correspond to TRANSLT's `-col`,
`-nc` and `-nh`. The reader accepts bare columns of numbers as well.

## `.json` - MantaRay native

The whole in-memory model, including regions, both calibrations, descriptions,
acquisition mode and where the data came from, so it round-trips exactly. Tagged
with `"mantaray": "spectrum/1"`.

serde_json's `float_roundtrip` feature is enabled: without it a coefficient such
as `2.9858588845854683e-7` comes back one bit different.

## `.Lis` - list mode

ORTEC's list format is instrument specific, so MantaRay defines its own container
and can slice it by time exactly as the List Data Range dialog does.

```text
0   8    magic "ORTSLIST"
8   2    u16 version (1)
10  2    u16 flags (reserved)
12  4    u32 channel count of the histogram
16  8    f64 live time in seconds
24  8    f64 real time in seconds
32  4    u32 event count
36  2    u16 detector number
38  2    u16 length of the sample description
40  ...  sample description, UTF-8
then     events: f64 time in seconds, u32 channel
```

## Nuclide libraries

ORTEC's binary `.Lib` reads exactly as ULI, GammaVision and MAESTRO write it.
The layout is section 6.1 of the *ORTEC Software File Structure Manual* (part
753800): 128-byte records, a header, then nuclide records (21 words, three to a
record) and peak records (16 words, four to a record). Three things the manual
does not say, established against the shipped example libraries:

- **R\*4 is an IEEE 754 float, little-endian** (Be-7's half-life reads as
  53.29 days in no other encoding), and the yield is already per hundred
  decays: Co-60 stores 99.97, not 0.9997.
- **File order lies.** Records are chained by ordinal number through fore
  pointers, deleted records stay in place on a free list, and after an edit the
  chain can start anywhere - `Mixed Gamma.Lib` ships with its first nuclide
  ninth on disk. The chain, not the disk order, is what GammaVision displays.
- **A peak's kind bits may all be zero** in old ULI files (GvDemo.Lib is from
  1988); such a line is a gamma. Bit 5 marks the key line, bit 6 keeps a line
  out of the average, matching the library editor's checkboxes.

Libraries MantaRay writes are stored as JSON (lossless) or as a CSV table that a
spreadsheet can produce:

```csv
nuclide,half_life_s,uncertainty_percent,flags,energy_kev,yield_percent,photon,key_line,not_in_average
Co-60,166344000,0.1,T.......,1173.228,99.85,G,1,0
Co-60,166344000,0.1,T.......,1332.492,99.9826,G,1,0
```

A four-column form (`nuclide,half_life_s,energy_kev,yield_percent`) is also
accepted. Rows are grouped by nuclide, keeping the order of first appearance,
because reports follow library order.

## Formats not implemented

- **`.Clb`** - a 768-byte ORTEC calibration file. One sample was available; the
  gain float is identifiable at offset 0x98 (0.36605 keV per channel, matching the
  `.Spe` files from the same detector) but a single file is not enough to decode
  the rest safely. Send more samples and it can be finished.
- **`.Cfg`, `.Cxt`** - detector configuration and context files. MantaRay keeps its
  own detector list and settings as JSON.
- **`.Cnf`** (Canberra Genie) and **IEC-1455** - not ORTEC formats, but both are
  common in the field and would fit the same codec layout.
