# Talking to real ORTEC hardware

What is known about reaching an ORTEC MCB from MantaRay, established from a
working MAESTRO Pro 9.01 installation and from the instrument on the bench.
Every claim here is marked **verified** (read out of a file, a binary or a
screenshot) or **unverified** (inferred, or taken from a secondary source and
not yet confirmed against the hardware).

## The instrument

**Verified.** An ORTEC 926 ADCAM MCB, connected by USB through ORTEC's DPM-USB
adapter. Windows enumerates the adapter as:

```
USB\VID_0A2D&PID_0016\11217584   bus-reported name "DPM-USB"
CompatibleID  USB\COMPAT_VID_0A2D&Class_00&SubClass_00&Prot_00
```

Device class 00 is vendor-specific, so it needs a vendor driver; there is no
inbox Windows driver that will bind it. On a machine without ORTEC software
installed it sits at **Problem 28, `CM_PROB_FAILED_INSTALL`** - no driver - and
nothing can reach it, MantaRay or otherwise.

MAESTRO's own status bar names the model: `Mcb Model No. 0926-001`.

## What the names mean

**Verified.** A detector's name is configuration, not hardware. The MAESTRO
machine has two 926s and one networked MCS:

| # | Name | Address | Type | Serial |
|---|---|---|---|---|
| 1 | `Low_Background` | `337 1 -1` | `0926-001` | *(empty)* |
| 2 | `Rayleigh` | `340 1 -1` | `0926-001` | *(empty)* |
| 3 | `XI MCSp-001 SN 0000` | `121 1 -1 192.168.1.106` | `MCSp-001` | `0000` |

from `MCBCON32.INI`, section `[DETS]`. "Rayleigh" is a label somebody typed
into MCB Configuration; the instrument does not report it, and the serial-number
field is empty for both 926s. The first field of `ADDR` (337, 340) is the
instrument number the local layer addresses; the trailing field carries an
address only for networked instruments.

The same names appear again in `MCBLOC32.INI` as `@DESC=` under per-instrument
sections (`[M337S01]`, `[M340S01]`), alongside that instrument's calibration.

## Where MAESTRO keeps things

**Verified.**

- `MCBCON32.INI` - the detector pick list: number, name, address, type, features.
- `MCBLOC32.INI` - per-instrument state, and in `[CONFIG]` the list of transport
  add-in DLLs that the local layer loads:

  ```
  ADDIN01=...\ORTEC Shared\UMCBI\USBAddin.Dll
  ADDIN02=...\ORTEC Shared\UMCBI\DpmUsbAddIn.dll      <- the DPM-USB adapter
  ADDIN03=...\ORTEC Shared\UMCBI\DigibaseEAddIn.dll
  ADDIN04=...\ORTEC Shared\UMCBI\RS232Addin.dll
  ADDIN05=...\ORTEC Shared\UMCBI\OasisNewAddIn.dll
  [INSTALL] DPM=1 PCI=1 PP=1 RUNSERVER=1
  ```

- Both live in `C:\Program Files (x86)\Common Files\ORTEC Shared\UMCBI\`.

The calibration stored there is the live one. `[M340S01]` (Rayleigh) carries
`EnergyCalibration=15.7835 0.359327 2.086E-008`, and MAESTRO's marker readout in
a running session showed channel 3874 = 1408.13 keV. That polynomial evaluated
at 3874 gives **1408.13 keV exactly**, so the file is not a stale copy.

*(One inconsistency worth knowing: `EntTable1=1780.0270 661.7000 4.4843` records
a 661.7 keV line at channel 1780.027, but the current calibration puts that
channel at 655.46 keV. The entry table predates a refit. Do not use the entry
table as a calibration source.)*

## The API

**Verified by parsing the binary.** `mcbloc32.dll` is **32-bit x86** and exports
43 functions, all `__stdcall` - the decorated names carry the stack byte count,
which is what `@N` means:

| Export | Bytes | Words | What it is |
|---|---|---|---|
| `_LocalStartup@0` / `_LocalCleanup@0` | 0 | 0 | open and close the layer |
| `_LocalGetConfigMax@8` | 8 | 2 | how many detectors are configured |
| `_LocalGetConfigName@24` | 24 | 6 | a detector's name |
| `_LocalGetDetectorInfo@28` | 28 | 7 | type, features |
| `_LocalMCBComm@40` | 40 | 10 | **the ASCII command channel** |
| `_LocalMCBGetData@36` | 36 | 9 | **read spectrum** |
| `_LocalMCBSetData@32` | 32 | 8 | write spectrum |
| `_LocalSetROI@36` / `_LocalClearROI@36` | 36 | 9 | regions |
| `_LocalIsActive@8` | 8 | 2 | is it counting |
| `_LocalGetStartTime@12` | 12 | 3 | acquisition start |
| `_LocalGuessMasks@16` | 16 | 4 | data and ROI bit masks |
| `_LocalLockDevSeg@20` / `_LocalUnlockDevSeg@16` | | | device-segment locking |

The PDB also names `lpdwROIMask`, matching the documented behaviour that a
spectrum word carries its ROI flag in a high bit and the mask says which.

**The bitness is the design constraint.** A 32-bit in-process DLL cannot be
loaded by a 64-bit process. MantaRay is 64-bit, so the instrument has to be
reached through a **small 32-bit sidecar process** that owns the DLL and speaks
to MantaRay over a pipe or a socket. That is not a workaround to be avoided later
- it is the shape of the solution, and the [`Transport`] seam already accepts
it, because a transport only has to carry a command out and a line back.

The call chain is:

```
mantaray (64-bit) ──pipe──> sidecar (32-bit) ──> mcbloc32.dll
                                                └─> DpmUsbAddIn.dll ──> [USB driver] ──> 926
```

## The driver

**Verified.** `ortecusb3.inf` is a customized build of **Cypress Semiconductor's
CyUSB** driver (KMDF 1.15), provided by ORTEC, and it names our device
explicitly in both the x86 and the amd64 sections:

```
; DPM-USB
%VID_0A2D&PID_0016.DeviceDesc%=ORTECUSB3, USB\VID_0A2D&PID_0016
...
VID_0A2D&PID_0016.DeviceDesc="DPM-USB"
```

which is exactly the bus-reported name Windows shows for the unplugged-looking
device. `ortecusb3.sys` is **x64**, so it loads on 64-bit Windows 11; the
service is `ORTECUSB3`, demand-start, with `MaximumTransferSize` 4096 and
device-interface GUID `{02C10BA3-54E3-4bb0-9BBF-A848AE18116A}`.

That it is a CyUSB derivative matters: the kernel driver is a generic bulk pipe,
and all the instrument protocol lives in user mode in `DpmUsbAddIn.dll`.

## The API, in full

**Verified from ORTEC's own header**, `mcbcio32.h` (518 lines), which shipped
with the installation. Every signature below is copied from it, not inferred.
`WINAPI` is `__stdcall`, and `HDET` is `void *`.

```c
BOOL    MIOStartup(void);
BOOL    MIOCleanup(void);
HDET    MIOOpenDetector(int nDet, LPCSTR lpszListName, LPCSTR lpszAuth);
BOOL    MIOCloseDetector(HDET hDet);

BOOL    MIOComm(HDET hDet, LPCSTR lpszCmd, LPCSTR lpszAuth, LPCSTR lpszPass,
                int nMaxResp, LPSTR lpszResp, LPINT lpnRespLen);

LPDWORD MIOGetData(HDET hDet, WORD wStartChan, WORD wNumChans,
                   LPDWORD lpdwBuffer, LPWORD lpwRetChans,
                   LPDWORD lpdwDataMask, LPDWORD lpdwROIMask, LPCSTR lpszAuth);
LPDWORD MIOSetData(HDET hDet, WORD wStartChan, WORD wNumChans,
                   LPDWORD lpdwBuffer, LPCSTR lpszAuth);

BOOL    MIOGetConfigMax(LPCSTR lpszListName, LPINT lpnDetMax);
BOOL    MIOGetConfigName(int nDet, LPCSTR lpszListName, int nNameMax,
                         LPSTR lpszName, LPDWORD lpdwID, BOOL *lpbOutDated);
BOOL    MIOGetDetectorInfo(HDET hDet, LPSTR lpszDesc, int nMaxDesc,
                           BOOL *lpbDefaultDesc, DWORD *lpdwID, BOOL *lpbDefaultID);
BOOL    MIOIsActive(HDET hDet);
long    MIOGetStartTime(HDET hDet, long *lpCurrentTime);
BOOL    MIOSetROI  (HDET, WORD wStartChan, WORD wNumChans, LPCSTR lpszAuth, LPCSTR lpszPass);
BOOL    MIOClearROI(HDET, WORD wStartChan, WORD wNumChans, LPCSTR lpszAuth, LPCSTR lpszPass);
BOOL    MIOLockDetector(HDET, LPCSTR lpszAuth, LPCSTR lpszPass, LPCSTR lpszOwnerName);
BOOL    MIOUnlockDetector(HDET, LPCSTR lpszAuth, LPCSTR lpszPass);
int     MIOGetLastError(LPINT lpnMacroErr, LPINT lpnMicroErr);
LPCSTR  MIOGetTypeEx(HDET hDet, LPSTR lpszType, int nMaxType);
BOOL    MIOIsFeature(HDET hDet, int nFeature);
```

Notes worth carrying into the binding:

- The two string parameters that third-party code passes as `""` are
  **`lpszAuth` and `lpszPass`** - authorisation and the detector password.
- `lpdwDataMask` and `lpdwROIMask` are **outputs**: the driver reports which
  bits of each 32-bit channel word are counts and which flag an ROI. Use the
  returned masks rather than assuming the ROI flag is bit 31.
- `wStartChan`/`wNumChans` are `WORD`, so **65535 channels per call** - ample
  for the 926's 8192.
- `MIOComm2` does **not** appear in the header. It is not part of this API.
- Errors: `MIOENONE 0`, `MIOEINVALID 1`, `MIOEMCB 2`, `MIOEIO 3`, `MIOEMEM 4`,
  `MIOENOTAUTH 5`, `MIOEBLOCKING 6`, `MIOEINTR 7`, `MIOENOCONTEXT 8`,
  `MIOENOTOPEN 9`, `MIOEUNEXPECTED 10`, `MIOENOTSUPPORTED 11`, and negative
  macro errors `MIOEMACCLOSED -1` through `MIOEMACOTHER -5`.
- Feature bits are indices into `MIOGetFeatures`, `MIOFEAT_CONVGAIN 0` through
  `MIOFEAT_DISCINPUT 39` and beyond - `MIOFEAT_DPMADDR 29` is the one that marks
  a DPM-addressed instrument like ours.

## What we have

Everything needed to close the chain is now in hand:

| File | What it is | Bitness |
|---|---|---|
| `ortecusb3.inf/.sys/.cat` | the kernel driver, names `PID_0016` | **x64** |
| `Mcbcio32.dll` + `mcbcio32.h` + `.lib` | the documented API | x86 |
| `mcbloc32.dll` | the local layer under it | x86 |
| `DpmUsbAddIn.dll` | the transport add-in for this adapter | x86 |
| `USBAddin.dll`, `TCPAddin.dll`, `RS232Addin.dll` | the other transports | x86 |
| `MCBServerNX.exe`, `mcbcon32.exe` | MCB server, MCB Configuration | |
| `MCBCON32.INI`, `MCBLOC32.INI` | detector list and per-instrument state | |

The kernel driver being x64 while everything above it is x86 is the whole story
in one line: Windows can load the driver in a 64-bit system, but the DLLs that
speak to it can only be loaded by a 32-bit process.

## The command dialect

**Verified against the 926 itself**, by sending commands through `MIOComm` and
reading what came back. Every reply below is a real one.

| Command | Reply | Meaning |
|---|---|---|
| `SHOW_VERSION` | `$F0926-001` | the model |
| `SHOW_TRUE` | `$G0007892556117` | real time, in ticks |
| `SHOW_LIVE` | `$G0007663632108` | live time, in ticks |
| `SHOW_TRUE_PRESET` | `$G0000000000075` | no real-time preset |
| `SHOW_LIVE_PRESET` | `$G0012960000093` | live preset, in ticks |
| `SHOW_ACTIVE` | `$C00000087` | not counting |
| `SHOW_GAIN_CONVERSION` | `$C08192107` | 8192 channels |
| `SHOW_ROI` | `$D0399900044110` | a region at 3999, 44 channels wide |
| `SHOW_STATUS` | `$M000766363200078925560000000000092` | live, real, and a third counter |
| `SHOW_PEAK_PRESET` | `$G0000000123081` | ROI peak preset, in counts |
| `SHOW_INTEGRAL_PRESET` | `$G0000004567097` | ROI integral preset, in counts |

The last two rows - and the `SET_PEAK_PRESET` / `SET_INTEGRAL_PRESET` verbs
that write them, arguments in counts with no tick conversion - were established
over libusb on the Linux bench (2026-08-05): each value written read back
exactly. Two more things the same session established: a valid `SET` command
answers an **empty reply**, indistinguishable from an unknown verb (so
translation has to be right - the instrument will not say when it is not), and
a live-time preset set through the dialect really does stop the instrument by
itself, at exactly the preset value on the live clock.

**A preset outlives the session that set it, and the instrument will not
mention it.** On 2026-08-06 a 926 was found still holding
`SHOW_LIVE_PRESET` = `$G0000015000081` - 15000 ticks, 300.00 s - from an
earlier run, with its live clock stopped at exactly that. Starting it again
in that state is the trap: `START` is accepted, answers as usual, and the
instrument simply does not count, because the preset is already satisfied.
Nothing in the reply says so. That is why the bridge answers `SHOW_PRESETS`
by reading all four registers back (`SHOW_TRUE_PRESET`, `SHOW_LIVE_PRESET`,
`SHOW_PEAK_PRESET`, `SHOW_INTEGRAL_PRESET`), and why mantaray asks that
question when it connects rather than assuming an instrument holds whatever
this session last wrote. Zero in a register means none set, which is how the
instrument itself says it. Clearing the spectrum resets the clocks and lets
the same preset run again from the beginning - the preset itself survives a
`CLEAR`.

**The record format.** A reply is `$`, a letter naming the record, one or more
fixed-width decimal fields, then a three-digit checksum:

- `$C` - one 5-digit field.
- `$G` - one 10-digit field.
- `$D` - two 5-digit fields.
- `$M` - three 10-digit fields.
- `$F` - free text, for version strings.

The **checksum is the sum of every preceding ASCII byte, including the `$` and
the letter, modulo 256**, printed as three digits. Confirmed on two records:
`$C08192` sums to 363, and 363 - 256 = 107, the checksum shown; `$G0007892556`
sums to 629, and 629 - 512 = 117.

**The clock runs in twenty-millisecond ticks.** This is not a guess: 7892556
ticks is 157851.12 s, and MAESTRO reading the same instrument displayed
`Real: 157,851.12`. Live time and the live preset check the same way, against
`153,272.64` and `259,200.00`.

**What MantaRay gets wrong today.** `crates/mantaray-device/src/remote.rs` sends
`SET_PRESET_LIVE`, which the instrument **rejects** - the real command is
`SET_LIVE_PRESET`, and its argument is ticks, not seconds. `SHOW_STATUS` does
exist but returns a `$M` record, not the `RT=.. LT=..` text MantaRay's simulator
answers with. Correcting that is a separate piece of work; the bridge does not
depend on it, because it asks the instrument directly.

[`Transport`]: ../crates/mantaray-device/src/transport.rs

## Configuring it ourselves

**Verified against two instruments.** MantaRay does not need a configuration
inherited from anywhere. `mantaray-mcb configure` asks the transports what is
answering and writes both files the local layer wants.

The minimum `MCBCON32.INI` the library accepts, established by bisection:

```
[DETS]
MAX=2
NAME001=1 0926-001
ADDR001=337 1 -1
TYPE001=0926-001
SNUM001=
NAME002=2 0926-001
ADDR002=338 1 -1
TYPE002=0926-001
SNUM002=
[GENERAL]
DATE=1785537462
```

Two things about that are worth knowing, because neither is guessable:

- **`FEAT` lines are optional.** The local layer guesses an instrument's
  features from its type string when none are given - that is what
  `LocalGuessFeatures` is for.
- **The `DATE` line is not.** Without it every `MIOOpenDetector` fails, and it
  fails reporting *no error at all*, macro 0 and micro 0, which is the least
  helpful way anything has failed in this whole exercise.

**MCB numbers are assigned, not fixed.** With one adapter connected the
instrument answered at 337; plugging in a second gave 337 and 338. The
MAESTRO machine had the same two instruments at 337 and 340. So a number is a
position in an enumeration, and a configuration copied between machines will
point at the wrong instrument - which is exactly what happened here, and put a
spectrum's Eu-152 line at 107.6 keV instead of 121.78 until it was caught.

## Doing the USB in house

**Verified from `DpmUsbAddIn.pdb`.** The add-in that ORTEC's library loads to
reach this adapter is a thin thing, and what it sits on is ordinary:

- It uses **Cypress's CyAPI** directly - `CCyUSBDevice`, `CCyUSBEndPoint::XferData`
  - over the ORTECUSB driver, which is itself a customized CyUSB build. There is
  one I/O primitive, `DPMUSBCI::USBDeviceIO(CCyUSBDevice*, UDIOFunctionCode,
  const void*, DWORD, DWORD*)`, and its error strings say **Bulk**. No control
  transfers, no vendor setup packets.
- Above that is a **mailbox**: `mcb_wakeup`, `write_mbx`, `write_mbx_byte`,
  `read_mbx`, `read_mbx_byte`, `mailbox_type` returning an `EMBXTYP`. That is
  the dual-port-memory idiom - a command goes into a mailbox, a reply comes
  back out of one.
- On top of the mailbox rides `mcb_dialog(MCB=%d)`, which carries the ASCII
  dialect this document already decodes: `SHOW_LIVE` in, `$G0007663632108` out.
- Device discovery is `SetupDiEnumDeviceInterfaces` on `ORTECUSB2_GUID`, with an
  `USBADDINLRU.DAT` cache of what was seen before.
- Sibling classes `M918CI` and `M919CI` share the mailbox helpers, so the same
  framing serves several ORTEC instruments, not only the DPM-USB adapter.

So the layers are:

```
ASCII dialect ($G..., checksummed)   <- decoded, see above
        mailbox (wakeup, write, read)  <- decoded, see below
                bulk endpoints          <- ordinary USB, libusb reaches it
```

All three layers are now worked out, and everything below is what they turned
out to be. That matters for two things at once: MantaRay no longer depends on
ORTEC's user-mode software on Windows, and the same code runs on Linux and
macOS, where `Mcbcio32.dll` can never go and libusb can claim
`VID_0A2D&PID_0016` with no driver at all.

### What works already

**Verified against both adapters, 2026-07-31.** MantaRay holds a full conversation
with a 926 over USB with no ORTEC software in the process at all - no
`Mcbcio32.dll`, no `mcbloc32.dll`, no `DpmUsbAddIn.dll`. Only ORTEC's *driver*
is involved, which is the thing that was deliberately kept.

```text
> mantaray-mcb usbtalk --device 11217584 SHOW_VERSION
SHOW_VERSION -> "$F0926-001"

> mantaray-mcb usbtalk --device 08134076 SHOW_STAT
SHOW_STAT -> "$M000372850500037299040000000000081"

> mantaray-mcb usbdump --device 08134076 0x0400 32
0x0400  25 00 30 00 30 00 30 00 30 00 30 00 30 00 30 00  |%.0.0.0.0.0.0.0.|
0x0410  36 00 39 00 0d 00 35 00 31 00 30 00 35 00 0d 00  |6.9...5.1.0.5...|

> mantaray-mcb usbspectrum --device 08134076 --out live.json
8192 channels in 0.06 s: total 1643807, peak 2541 at channel 272
SHOW_INTEGRAL agrees: 1643807
```

Commands answer, the clocks read, the instrument's own memory can be read out,
and a whole spectrum comes back in one frame - **all 8192 channels identical to
what ORTEC's own library reads from the same instrument, with the clocks
matching to the millisecond**. Everything below is how that was reached, in the
order it mattered.

Two pieces came first, and neither needed guessing.

**Finding the device without SetupAPI.** Windows records every bound interface
under `HKLM\SYSTEM\CurrentControlSet\Control\DeviceClasses\{interface guid}`,
and the subkey name is the device's symbolic link with its separators escaped -
`##?#USB#VID_0A2D&PID_0016#11217584#{guid}` is the link
`\\?\USB#VID_0A2D&PID_0016#11217584#{guid}`. Reading four registry keys is the
whole of what the enumeration API would have done here.

**The control codes are published.** ORTEC's driver is a rebuilt Cypress CyUSB,
so its interface is Cypress's, documented in `cyioctl.h`:
`CTL_CODE(FILE_DEVICE_UNKNOWN, n, METHOD_BUFFERED, FILE_ANY_ACCESS)`, which
works out as `0x0022_0000 | (n << 2)`.

| n | code | what it does |
|---|---|---|
| 5 | `0x00220014` | how many endpoints |
| 8 | `0x00220020` | control transfer on endpoint zero |
| 9 | `0x00220024` | **bulk transfer**, a `SINGLE_TRANSFER` header then the payload |
| 11 | `0x0022002C` | reset a pipe |
| 15 | `0x0022003C` | the device's name |
| 17 | `0x00220044` | abort a pipe |

The full numbering was recovered from the driver binary itself: `ORTECUSB3.sys`
keeps its handler names in its tracing strings, in order -
`CyIoctlHandler_GetCyUSBDriverVersion`, `_GetUSBDIVersion`,
`_GetAltIntrfSetting`, `_SetInterface`, `_GetDeviceAddress`, `_GetNoOfEndpoint`,
and so on - and every code ORTEC's own add-in sends appears as a plain immediate
in `DpmUsbAddIn.dll`, which confirms the mapping without relying on either.

Codes above 20 belong to a later CyUSB than this 2017 driver, which answers them
with "buffer too small" whatever buffer is given.

Both adapters report **two endpoints**: `0x01` bulk out and `0x81` bulk in, both
64 bytes a packet, at full speed.

### Three bugs, and how each was found

Everything about the protocol was decoded correctly and *nothing worked* -
every transfer timed out, on both endpoint zero and the bulk endpoints. Three
separate mistakes were stacked on top of each other, and each hid the next.
They are recorded because each has the same shape: a plausible reading of a
structure, and a failure mode that says nothing at all.

**One: the header was two bytes too long.** `SINGLE_TRANSFER` was transcribed as
forty bytes, the size it would be with ordinary four-byte alignment. It is
**thirty-eight**, packed. The proof is in ORTEC's own code, which allocates
`0x26` bytes for it and writes `BufferOffset` at `0x1e` and `BufferLength` at
`0x22`:

```asm
lea  eax, [ebx + 0x26]              ; total = payload + 0x26
call malloc
mov  byte  ptr [esi + 0x0d], al     ; ucEndpointAddress
mov  dword ptr [esi + 0x1e], 0x26   ; BufferOffset
mov  dword ptr [esi + 0x22], ebx    ; BufferLength
```

which pins the layout exactly:

| offset | field |
|---|---|
| 0 | setup packet: `bmRequest`, `bRequest`, `wValue`, `wIndex`, `wLength` |
| 8 | `TimeOut`, **in seconds**, zero meaning the driver never gives up |
| 12 | `WaitForever` |
| 13 | `ucEndpointAddress` |
| 14 | `NtStatus` |
| 18 | `UsbdStatus` |
| 22 | `IsoPacketOffset` |
| 26 | `IsoPacketLength` |
| 30 | `BufferOffset` |
| 34 | `BufferLength` |
| **38** | the payload |

With the header two bytes too long, the driver read `BufferOffset` and
`BufferLength` out of the wrong bytes, asked the controller for a transfer that
could not happen, and simply never completed. No error, no stall - the request
sat in the queue until the timeout withdrew it. A test now pins the layout
field by field so it cannot drift back.

**Two: a read has to offer whole packets.** A sixteen-byte read into a
sixteen-byte buffer never completes, even with the header right. The endpoint's
packet size is 64, and the driver wants somewhere to put a whole packet whether
or not the instrument fills it. Rounding the receive buffer up to a multiple of
64 - and only the buffer, not the request - fixes it. The instrument still sends
exactly what was asked for.

**Three: a read frame must not be padded.** This one produced the strangest
failure in the whole exercise. ORTEC's read frames carry a stray byte after the
six-byte header, so padding a read out to the length being asked for looked
harmless. It is not: the adapter takes its frame from the first six bytes and
then **reads the padding as further frames**, and answers those too. Nothing
fails. Every reply is a real reply - to a question asked several exchanges ago.
`SHOW_TRUE` answers with the version string; `SHOW_LIVE` answers with the true
time. A read frame is now the header alone, six bytes, and nothing else.

This is the failure worth remembering, because it is the one that survives
testing: an instrument answering confidently and wrongly looks like a working
instrument.

### Recovering an adapter, without the cable

An abandoned transfer leaves the adapter holding a reply nobody collected, and
from then on every answer is one question behind. Left alone this needs a
physical replug, which is not a thing to ask of anyone using the program.

Three levels of recovery are implemented, tried in order by `mantaray-mcb usbfix`:

1. **Abort and reset both pipes**, then read the in endpoint until it stops
   answering. This drains queued replies and is enough for most slips.
2. **Cycle the port** - `IOCTL_ADAPT_CYCLE_PORT`, which is a replug in software.
   The device re-enumerates under the same path, because the path carries the
   serial number. Verified to bring back an adapter that nothing else would
   revive.
3. If neither works, say so plainly: the cable and the instrument's power are
   what is left, and those are things only a person can check.

Both of the first two come with a caveat learned the hard way, and neither is
free:

- **Draining wedges a healthy adapter.** Running step 1 on an adapter that was
  answering leaves it answering nothing. It is a repair for a stream already
  known to be broken, not a precaution, and it is no longer run automatically
  after a timeout.
- **A replug in software can fail to plug back in.** Step 2 takes the device
  off the bus, and it returns only when Windows enumerates it again. Usually
  that happens within a second; once here it did not happen at all, and the
  adapter stayed missing until it was physically unplugged and plugged back in.
  `pnputil /scan-devices` would also do it, but that needs an Administrator.
  So `usbfix` drains by default and cycles only when asked, with `--cycle`.

The check is deliberately stricter than "did it answer": after the slip above, a
wedged adapter answers everything. `usbfix` asks for `SHOW_VERSION` twice and
requires a `$F` record both times.

### The mailbox, decoded

**Verified from a USBPcap capture** of ORTEC's own library driving the 926, read
against commands whose replies were already known. Two bulk endpoints, and
three opcodes.

**A command goes out on endpoint 0x01** as a six-byte header and the ASCII:

```
0a <len> 00 08 04 02  <command bytes>
```

`0a` is the block write, `<len>` is how many ASCII bytes follow, and
`00 08 04 02` is fixed. Observed exactly:

```
0a 0c 00 08 04 02  "SHOW_VERSION"     len 0x0C = 12
0a 09 00 08 04 02  "SHOW_CONF"        len 0x09 = 9
0a 0a 00 08 04 02  "SET_SEGM 1"       len 0x0A = 10
```

The short forms are ORTEC's own abbreviation rule, not truncation: the library
sends `SHOW_CONF` and `SHOW_VERS` where the manual writes the full word.

**The reply comes back on endpoint 0x81** as a little-endian length, then the
ASCII, ending in a carriage return:

```
0b 00  "$F0926-001" 0d        len 0x000B = 11 bytes
60 00  "$J08192000010819..."  len 0x0060 = 96 bytes
00 00  00 00                  nothing to say (SET_SEGM)
```

That ASCII is the dialect already decoded above - `$F` a version, `$J` a
configuration, `$G` a clock - so the two halves meet here.

**The one-byte traffic around it is the mailbox itself**, and it behaves exactly
as `DpmUsbAddIn.pdb` named it:

```
03 01 00 08 <addr> 00 <pad>        read one byte from <addr>   -> one byte back
02 02 00 08 <addr> 00 <d0> <d1>    write bytes to <addr>       -> 02
```

Only addresses `0x02` and `0x04` are used. A write of `5a` to `0x02` followed by
a read of `0x02` gives `5a` back, which is what settles that these are memory
rather than commands. `0x5A` and `0xA5` are passed back and forth as a
ready handshake before and after each block.

### One frame, three opcodes

**Verified against the instrument.** Every frame is the same six-byte header,
which is what makes the whole thing small enough to reimplement:

```
<op> <len:u16 LE> <space> <offset:u16 LE>  [data...]
```

| op | what it does | data | the answer |
|---|---|---|---|
| `0x03` | read `len` bytes | none | the bytes |
| `0x02` | write | the bytes | one byte |
| `0x06` | read a block of `len` bytes | none | the bytes |
| `0x05` | write a block | the bytes | one byte |
| `0x0A` | hand an ASCII command over | the ASCII | `<len:u16 LE>` then the text then `0d` |

Those five are all of them: eleven places in `DpmUsbAddIn.dll` build a frame,
and every one loads the space byte from the same global, so the list can be had
by finding its references rather than by guessing.

The length is sixteen bits, little-endian - what looked like a constant `00 08`
is the length's high byte, always zero for a frame that carries fewer than 256
bytes, followed by the address space. Opcodes `0x03` and `0x06` differ only in
where the length comes from: ORTEC's read-one builder stores a hard `1`, its
block builder stores the caller's count. `0x06` is what makes a spectrum-sized
read possible at all - the whole 8 KB mailbox window comes back in eight frames
rather than thirty-three.

`<offset>` is a flat sixteen-bit byte address into the space, little-endian -
the command buffer at `0x0204` goes on the wire as `04 02`, which is the thing
most easily got backwards. ORTEC's own callers pass a *word* address and the
library doubles it, which is visible as the `lea eax, [ecx + ecx]` in every
builder.

**One packet of slack on every read.** A device that has sent a whole number of
64-byte packets marks the end with an empty one. A receive buffer sized exactly
to the answer leaves that terminator behind, the next read collects it and
returns nothing, and every reply after that is one question late - which reads
as an instrument that answers confidently and wrongly. Asking for one packet
more than is wanted lets the driver swallow it where it belongs.

**Clearing the pipes is not a repair.** `settle()` - abort, reset, drain - will
wedge an adapter that was working. It is worth doing only when the stream is
already known to be broken. Cycling the port is a replug in software and is a
genuine last resort: the adapter leaves the bus and comes back only when
Windows enumerates it again, which an ordinary user cannot make happen. One
adapter here has been sitting unplugged-in-software since. `usbfix` therefore
drains by default and cycles only when asked with `--cycle`.

### The dual-port memory, as surveyed

**Verified by reading it.** `mantaray-mcb usbdump <offset> <length>` walks the
address space, `usbscan` walks the whole of it, and `usbspaces` asks every one
of the 256 values byte three can take.

Byte three of the frame is not part of a constant: it names an **address
space**. ORTEC's library fills it from a global it sets at start-up
(`DpmUsbAddIn.dll+0x20fa4`, zero in the file), and a DPM-USB in front of a 926
gets eight. Of the 256 values only bits 0 and 3 do anything, so there are four
spaces, and only two hold anything: **space 8, the mailbox**, and **space 0,
the spectrum**.

Space 8 is 8 KB and wraps at `0x2000`:

| offset | what is in it |
|---|---|
| `0x0000` | the mailbox: the handshake byte at 2, the request register at 4 |
| `0x0020`-`0x0060` | the last command and its parameters, as ASCII |
| `0x0204` | where a command is written |
| `0x0400` | the last reply and the status records, as ASCII |

The reads interleave two banks - even addresses returning one page and odd
another, which is what the alternating `1e`/`e1` filler is.

**The spectrum is not in this window, and that is settled rather than guessed:**
reading all 8192 channels through ORTEC's own library leaves the whole 8 KB
byte-for-byte identical, and no run of channel counts correlates anywhere in
it. There is no paging register to find, and there is no other space to look
in - the sweep found four and two are empty.

### Reading a spectrum

**Solved, and verified channel for channel against ORTEC's own readout.** The
histogram lives in address space `0x00`, four bytes to a channel, starting at
offset zero. One frame fetches the whole thing:

```
03 <4*channels : u16 LE> 00 00 00        read the histogram
```

That is exactly what MAESTRO sends - taken off the wire while it refreshed its
display - and there is no paging, no handshake and no second request. The
length field is sixteen bits, which is precisely enough for the 32 KB an
8192-channel spectrum takes, and 8192 channels come back in about a sixteenth
of a second.

Two things make the difference between counts and nonsense:

- **The top bit of each channel word marks a region of interest, not counts.**
  A spectrum read without masking `0x8000_0000` off has channels holding two
  billion counts, which is what an ROI looks like when it is mistaken for a
  number. Masked, the low thirty-one bits are the count and the bit itself
  gives the instrument's ROI list for free.
- **Ask the driver for the bytes a few thousand at a time.** A single
  request for a whole spectrum never completes; ORTEC's own readout arrives in
  512-byte transfers.

Take the channel count from the instrument rather than assuming it: `$C` from
`SHOW_GAIN_CONVERSION` carries the conversion gain in its first five digits,
and an instrument set to 4096 channels answers with a 16 KB spectrum, not a
32 KB one.

`mantaray-mcb usbspectrum --out FILE` does this and checks itself, summing the
channels and comparing against the instrument's own `SHOW_INTEGRAL` over the
same range. A disagreement is a failure, not a warning.

Note that `0x03` and `0x06` are not interchangeable. `0x06` reads a *block* and
hands back every **other** byte of the memory it covers - which for a histogram
means bytes 0 and 2 of each count, a plausible-looking spectrum missing bits
8-15. `0x03` returns consecutive bytes. Both take a sixteen-bit length; only
one of them reads a spectrum.

### Driving it from the application

**Verified against the instrument.** `mantaray-mcb serve` speaks MantaRay's own
dialect on a pipe, and it now reaches the instrument **over USB first**, falling
back to ORTEC's library only when that cannot. The order is the point: the USB
path needs nothing but the kernel driver, so a machine that has never had
MAESTRO installed drives its own detector. `--usb` or `--umcbi` insists on one
or the other, which is what a person debugging one of them wants.

Both routes implement the same `Instrument` trait, so the translation above them
- clocks in ticks, presets in seconds, dead time from the two clocks - is
written once.

`crates/mantaray-device/tests/bridge_hardware.rs` drives the whole chain:
MantaRay's dialect, through the bridge process, over USB, to a real 926. It
checks that the channels sum to the total the instrument itself reports, and
that `START` and `STOP` move it. It needs hardware, so it skips out loud without
`MANTARAY_MCB` naming a bridge, and it takes a lock because an adapter can be
open in one process at a time.

Two things that were quietly wrong and are worth not repeating:

- **The device path was printed on standard output.** In `serve`, output *is*
  the protocol, so the first thing the application read was a path rather than a
  reply. Diagnostics belong on standard error.
- **Windows keeps an interface record for every adapter ever bound**, so the
  list outlives the cable. Unasked, take the first that actually opens rather
  than the first recorded, or a serve session picks an adapter that left months
  ago.

### Away from Windows

**Verified on Linux, 2026-08-05.** There is no vendor driver on these platforms
and none is needed: the kernel's own USB stack hands the interface over, and
the same frames go down the same two bulk endpoints.
`crates/mantaray-mcb/src/direct.rs` is that path, on libusb through `nusb`;
`crates/mantaray-mcb/src/dpm.rs` sits on a `BulkDevice` trait so the protocol is
written once and neither platform is a special case.

The path met an instrument for the first time on a Linux desktop with a 926 on
adapter `08134079`, and every assumption recorded above held on first contact:

- enumeration found the adapter by `0a2d:0016` and read its serial;
- `SHOW_VERSION` answered `$F0926-001`, and the clocks answered `$G` records
  whose tick arithmetic matched (`$G0000009496103` -> 189.92 s real,
  `$G0000009193097` -> 183.86 s live);
- a whole 4096-channel spectrum read out with its ROI bits, 284 345 counts,
  in about a fifth of a second with process start-up included;
- `mantaray-mcb serve` carried the same instrument into the desktop
  application - Scan, Open all, and a live detector window - through the same
  `Session` translation the Windows bridge uses.

What is **still unverified**: macOS, which type-checks and nothing more; and
multiple adapters on one Linux bus (the bench has one).

The permission prediction below also held exactly - the first open failed with
`errno 13` and nothing else was missing. The rule that fixed it:

```
SUBSYSTEM=="usb", ATTR{idVendor}=="0a2d", ATTR{idProduct}=="0016", TAG+="uaccess", MODE="0660"
```

in `/etc/udev/rules.d/70-ortec-dpm-usb.rules`, then reload, trigger and replug.
If opening the adapter fails on Linux it is almost always this rather than
anything missing - permission, not a driver.

### What is left

Nothing for the instrument in hand on Windows: commands, clocks, configuration,
gain, mode, integrals, the dual-port memory, whole spectra and the application
itself all work with none of ORTEC's user-mode software.

Two things remain, and neither is protocol work:

1. **Run the libusb path against hardware** on macOS. On Linux this is done -
   see "Away from Windows" above; on macOS it compiles and nothing more is
   claimed.
2. **Windows without ORTEC's driver at all.** Binding the adapter to WinUSB
   instead would remove the last vendor dependency, and `nusb` already speaks
   that. The cost is that ORTEC's own software stops seeing the device while it
   is bound, so it is a choice rather than an improvement - on a machine with no
   MAESTRO installed, it costs nothing.

What was ruled out along the way, so it is not tried again: a paging register
in the mailbox window (the window does not change across a full readout);
another address space for the counts (only four exist and two are empty); and
the `WRITE`/`GO` handshake from the 920E manual, which is a serial-line
protocol the 926 does not answer over DPM - ORTEC's own library fails it with a
communication error too.
