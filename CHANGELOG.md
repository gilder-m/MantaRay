# Changelog

## Unreleased

**A region is fitted as a whole, and told how many lines are in it
(2026-08-14).** Double-clicking a peak now asks whether it is one peak. The
whole region is fitted at once - several Gaussians sharing one width, on one
continuum, with a Compton step and a low-energy tail - and the number of
lines is decided by the counts rather than assumed. Where a region turns out
to hold more than one, Peak Info reports each line's own centroid, width and
area, and the headline figures describe the strongest of them instead of the
midpoint of the bump, which is where no line is.

The peak search uses the same machinery, and it closes a gap that could not
be closed any other way. A matched filter's kernel is as wide as a peak, so
two lines closer than about one and a fifth of a width make a single
response - and the merge was self-sustaining, because the bump's width is
half again a real line's, the spectrum's width law learns *that*, and the
kernel built from it is wider still. Two lines a full width apart came back
as one peak 27 channels wide where the truth is 14. Fitting settles it
without reference to any kernel: on synthetic pairs the search now separates
them from **0.6 of a width apart**, where the same test puts becquerel's
floor at 1.0 - and where the pair is four to one rather than even, becquerel
separates it at no separation tested at all, its weaker member never making
a maximum of its own. Both comparisons give becquerel the true resolution
law, which it requires and this does not.

On the fourteen-spectrum bench corpus this changes exactly one result, and
it is a real one: Ba-133's 79.6 and 81.0 keV pair, which every previous
version reported as a single line, now comes back as two - 0.12 and 0.13 keV
from their book values, in a 7.8% intensity ratio against the evaluated
7.7%. Nothing else on the corpus moves.

The arrangement follows [InterSpec](https://github.com/sandialabs/InterSpec)
(Sandia National Laboratories), rewritten in Rust: the amplitudes, continuum
and step are linear given the centroids and width, so they are solved exactly
rather than searched for, and the peaks in a region share one width. Both
matter - a doublet is a three-parameter search rather than a nine-parameter
one, and free widths would let a fit explain one fat peak as a narrow spike
on a broad pedestal.

Held back deliberately, and measured rather than assumed: a split has to
survive a chi-square improvement of 40, a companion has to hold at least a
twentieth of its region, the parts have to be narrower than the bump they
came from, and no peak may sit within a width of the region's edge. Each of
those was added because something real got past its absence - a clean Ba-133
356 keV line offered a companion a fiftieth its size, the discriminator
cut-on at the bottom of every spectrum offered three, the overflow wall at
the top offered three more. The one limit that cannot be tuned away: a
detector tailing worse than about 8% will have its strongest lines split,
because at that point the counts genuinely do not distinguish a tail from a
companion. At 4% - worse than a detector in reasonable health - none of
forty trials split.

One region rule serves every caller, because the search and the double-click
only agree if they are asked about the same channels - with the search
looking at forty channels and the double-click at thirty, the same overflow
artefact came back as three pieces to one and two to the other. Three widths
each side, never less than twenty channels: sized by what could overlap the
peak rather than by where the peak ends, because a companion must stand a
full width inside the window before the fit will believe in it, and a window
drawn just wide enough to contain a neighbour puts that neighbour exactly
where it will be disbelieved. Double-clicking a peak with nothing selected
therefore measures over a wider region than before, which also puts the
background channels of equations (17)-(21) well clear of the peak's
shoulders.

`peak_info` is unchanged and still cheap; the fitting lives in the new
`resolved_peak_info`, so the region tables and the acquisition presets that
call it for every region on every frame pay nothing. Peak search itself now
costs about a third of a second on an 8,192-channel spectrum where it cost
twenty milliseconds - it is fitting every peak it finds - and a region whose
single Gaussian already explains it for less than the splitting threshold is
skipped outright, which cannot change any result and is over half of them.
The CLI's `peaks` command gains `--resolve`.

**The peak search knows what a peak should look like (2026-08-13).** Peak
search is now Becquerel's resolution-matched filter - the method of the
Lawrence Berkeley National Laboratory Applied Nuclear Physics group's
[becquerel](https://github.com/lbl-anp/becquerel) package, reimplemented in
Rust with the maintainers' permission and our thanks. One zero-sum kernel
whose width follows the detector's own resolution replaces the nine-scale
second-difference ladder: a one-channel spike where real peaks run fifty
channels wide scores nothing against a fifty-channel kernel, so the narrow
statistical spikes the ladder kept reporting are structurally impossible
rather than merely thresholded. The width comes from the spectrum's shape
calibration when it holds one; otherwise the spectrum teaches itself its own
width law from a trial-ladder bootstrap whose per-peak widths are measured
directly off the counts, each side of a peak judged against its own floor.

Proven on the bench detector's HPGe spectra: Ba-133's five signature lines
to within half a keV plus its sub-percent 160.6 and 223.2 keV lines, Co-60's
pair, and all eleven canonical Eu-152 lines with every additional detection
mapping to a real weak Eu line - including 1085.9 and 1089.7 keV resolved as
neighbours. The sensitivity dial keeps its meaning: it is the signal-to-noise
a peak must reach, in the same Poisson sigmas as before. Known blemish,
recorded honestly: on a background spectrum's nearly empty tail the
self-taught law can pass a few sparse-count artefacts; a trustworthy shape
calibration removes them.

**A shape calibration is audited before it is trusted (2026-08-14).** A
`$SHAPE_CAL` block rides along in real files whether or not anyone
calibrated shape - the bench corpus carries a constant 1.6 channels from
some other day on a 2.8-day background, and a quadratic that turns downward
and crosses zero at channel 4,800 of 8,192 on the Eu-152 spectra. Trusted
literally, the first rejected Tl-208 at 2,614 keV through the width gate
while reporting the ADC overflow wall as four peaks, and the second read
the 1,408 keV line's area four times low. Now the search always measures
the spectrum's peak widths off the counts first and keeps the calibration
only when most measurements agree with it - a lying calibration is outvoted
by its own counts. A calibration is also read only across the channels it
actually described: a fit that turns downward has said nothing about the
channels past its vertex, so past that point the spectrum's own measured
growth rate carries it on. This also serves detectors whose files never
carry an honest shape calibration at all: NaI(Tl) and friends get the same
self-taught width law as an uncalibrated spectrum.

The measuring pass itself learned three lessons from that background. A
peak's width is now remeasured with a window matched to the peak until the
number stops shrinking, because a wide search window on a sloping continuum
reads its floors far down the slope and taught the law widths a quarter too
wide. The law is first fitted as a weighted median of pairwise slopes, each
pair carrying the weight of its weaker end: a least-squares fit let the 511
keV annihilation line - genuinely Doppler-broadened and very loud - drag the
law up until real peaks failed the width gate against it, and an unweighted
median let a crowd of five-sigma background wiggles measuring noise outvote
the two 490-sigma lines a Co-60 spectrum actually has. A slope is a
statement about two peaks, and it knows no more than the fainter of them.
And a candidate whose curvature estimate reads too narrow gets
one appeal to the counts: a direct half-maximum measurement inside the gate
band keeps it, which returned the Pb-214 295 keV and Ac-228 338 keV lines a
drift-broadened long acquisition had pushed out, while a one-channel spike
measures exactly the clamp of that measurement and stays out. The peak cap
rises from 40 to 100: the bench background carries about fifty genuine
lines, and a cap that returns the strongest 40 was silently trading its
weakest real lines away. With all of it in place, that 2.8-day background
reports 52 peaks of which 49 are the natural chains line for line - Pb-210
at 46.5 keV through Tl-208 at 2,614.5 - and the three that are not sit
against the ADC overflow wall, named by their position.

The audit hears only witnesses that would stand in court: a cluster votes
against a calibration only when two rungs measured it directly, and fewer
than two such clusters cannot overrule at all. The presumption protects an
honest calibration on a spectrum whose one strong line is broadened physics
of its own - annihilation quanta, most commonly - at a known price: a
single-line spectrum with a wrong calibration keeps the wrong width, and a
line found under a claim narrower than the peaks is reported with the
claim's width and a net area estimate short in the same proportion.

**The search stops assuming germanium (2026-08-14).** Measured head to head
against becquerel 0.7.0 over thirteen spectra - a NaI detector, an ORTEC
digiBASE, a CsI D3S, a simulated CZT background, a mobile survey and eight
germanium spectra, 284 peaks verified by independent fits that neither
finder was allowed to vote on - three assumptions turned out to be about
germanium rather than about physics, and each cost real lines on everything
else.

A channel whose kernel ran off the end of the spectrum was skipped
entirely. That costs four sigmas of reach at each end, which is a dozen
channels on a germanium spectrum and seventy on a scintillator, and the
bottom of a scintillator spectrum is where Am-241 at 60 keV and Ba-133 at
81 keV live: the same synthetic peak was found at channel 500 and invisible
at channel 60. The kernel is now shortened there instead - symmetrically,
which is the part that matters, because an even zero-sum kernel is blind to
a straight line and a lopsided one is not. Lopsided, it reads a spectrum's
end wall as a peak and teaches the width law nonsense, which is what the
old refusal was really defending against.

A single measured peak was taken to mean that all of its width had
accumulated from nothing at channel zero - true enough of germanium, and on
a scintillator whose peaks run forty channels wide it predicts eighteen
channels at channel 60, losing a real forty-channel peak there. One
measurement now claims its own width and no growth rate.

And a constant `$SHAPE_CAL` was believed. No detector's resolution is
constant, so a single number can only be right somewhere; one real file
claims a flat 4.27 channels where its low lines measure 1.1 to 2.7, and
thirteen of them failed the width gate against it. A calibration inside the
gate's band is no longer automatically the best account of the peaks: where
the spectrum's own law explains both halves of the measured range better,
and the claim is out by more than a sixth, the law wins. Both halves,
because a law fitted to these peaks can be excellent where most of them are
and nonsense elsewhere - on one bench file a flat law beat an honest
calibration on average while running half again too wide at 300 keV, and
preferring it cost five real lines.

Across that corpus the search now finds 271 of 284 verified peaks against
becquerel's 277, with 57 unverifiable extras against becquerel's 120 - and
becquerel was handed the true resolution law for every file, which it
requires and this does not. Centroids land within 0.01 FWHM of the fitted
truth on germanium against becquerel's 0.05, since the peak is interpolated
between channels rather than reported as one. On the bench corpus the
change is close to neutral: Co-60 at 60 cm gains Bi-214's 1,120 keV line,
Eu-152 at 50 cm stops reporting a valley at 872.8 keV, and Ba-133 at 70 cm
loses a real 5-sigma Bi-214 line at 1,237.6 keV, which is not yet
explained.

**A spike loud enough to fool the filter is still not a peak (2026-08-14).**
The width gate reads the signal-to-noise map, and that map's Poisson
denominator is largest exactly where a tall spike is - which flattens the
map over the glitch and leaves its curvature reading a width near the
kernel's own. A 3,000-count spike on a 200-count floor measured 0.92 of a
nine-channel model, sat comfortably inside the gate, and was reported as a
peak: the one false positive this whole search was built to make
impossible. The counts either side settle it without needing a background
at all, since a peak three channels wide or more holds nearly as much in
each neighbour as at its apex. The accusation needs the statistics to
support it as well as the shape - at a handful of counts a neighbour falls
under half the apex often enough by chance, and demanding shape alone threw
real lines out of the bench corpus, Tl-208 at 2,614 keV among them. With
four sigmas of the apex's own noise required, the corpus loses no detection
at all and the spike still goes.

**Peak Info's background points stand clear of the peak (2026-08-14).**
An auto-marked region spanned three times the FWHM with the background
points taken out of that span, which put the innermost of them about two
sigma from the centroid - where a Gaussian still stands at a tenth of its
height. Equation (17) then read the background off the peak's own
shoulders, equation (20) returned a net area a tenth to a seventh short,
and the Gaussian fit, seeing a peak whose wings had been subtracted away,
reported every line narrower than the counts do. Three FWHM is where the
peak ends; the background points now get their own room beyond it. On the
bench corpus Co-60's 1,332 keV line goes from 14% short to 4%, Cs-137's 662
keV line from 10% to 2%, and Eu-152's 344 keV line from 14% to exact
agreement with a generous hand-drawn region - and every fitted width now
matches the measured one. Regions are wider by exactly the background
points they always needed.

Reading a calibration only as far as it was fitted is what the bench corpus
needed most. Thirteen of its fourteen files carry one downturned quadratic,
which turns over at channel 1,869 of 8,192; held flat across the remaining
five sixths of every spectrum it claimed 3.7 channels where those peaks
measure 4.7 to 5.4, and the audit passed it, because every peak that
vouched for it lived on the near side of the vertex. That quiet quarter of
a width cost real lines and real counts: Tl-208 at 2,614 keV and Bi-214 at
1,764 keV went unreported, and Co-60's 1,332 keV line was marked with a
region returning a third less than its net area. Continued instead at the
growth rate the counts teach, those files report those lines and read that
area to within a seventh of a generous hand-drawn region's - and the only
detection anywhere in the corpus that the change costs is a 0.1-sigma
fluctuation that the too-narrow kernel had been matching.

## 0.2.4-alpha (2026-08-13)

**A detector's calibration survives the session (2026-08-13).** Calibrating a
detector window wrote to a mirror that lives exactly as long as the
connection: the next session - or the next reconnect - rebuilt the mirror
from the configuration's `CAL` field, or from nothing, and the operator's
calibration was quietly gone. Reported from the bench as "calibration is not
being saved", intermittently, because whether it seemed saved depended on
what the operator did next. A calibration made on a detector is now filed
under the serial the instrument reports and put back at every connect,
outranking the configuration's own: the remembered one is what the operator
did, on this screen, since that file was written. Pinned end to end - two
sessions, one served instrument - and the memory follows the instrument, not
the session.

**A calibration can be saved to a file of its own (2026-08-13).** The `.Clb`
reader has a writer beside it now, and the Calibration dialog a "Save to
file..." next to its "Recall from file...": the road a calibration takes to
another machine without dragging a spectrum along. What is written is
exactly what reading understands - the six coefficients at the offsets four
known-good samples agree on - and it round-trips through the reader that was
checked against those samples. Whether MAESTRO itself accepts one has not
yet been tried against a Windows installation, and the writer's notes say so.

**A stopped instrument's clocks are read after the stop (2026-08-13).** The
bridge assembles a status from three separate instrument reads, and an
instrument counting out a preset could stop *between* them: clocks first and
the flag last, a reply could say "stopped, LT=0.98" about a one-second
preset the instrument had honoured to the tick - and the refusal that
depends on the preset being reached never fired. The flag is read first, so
a reply that says stopped carries clocks read after the stop. Caught by the
926 on the bench counting out a one-second live preset, and proven there
three runs in a row.

**The peak-shape fit reads the wings, weighted by what they know
(2026-08-13).** The Gaussian fit behind Peak Info fitted a parabola to the
logarithm of the net counts - correct for the shape, but blind below thirty
percent of the peak and unweighted above it. Blind low, it read a real
peak's width off its cap alone, which is why the drawn fit sat visibly
narrower than bench peaks; unweighted, the logarithm inflates exactly the
channels with the fewest counts, so the wings it did admit steered it
hardest. It now reaches down to a twentieth of the peak and weights each
channel by its own count - the variance of a counted channel's logarithm is
about one over the count - so the wings inform the width without deciding
it. Pinned by a clean Gaussian recovered to its own parameters and by
deliberately mangled wings that no longer move the answer.

**The linear axis earns the same care the logarithmic one had
(2026-08-13).** On automatic linear scaling the full scale was exactly the
tallest visible channel: the peak sat clipped flat against the frame, over
quarter gridlines labelled with quarters of an arbitrary count - 40737 made
gridlines at 10184 and 20368. The automatic axis now climbs the same 1-2-5
ladder the logarithmic one does, so the peak clears the frame and the labels
are quarters of a round number. A fixed linear scale stays exactly where the
operator put it.

**Zooming centres the marker (2026-08-13).** Zoom kept the marker at
whatever fraction of the window it already occupied, so zooming toward a
peak near the edge kept it pinned at the edge - each step giving the marked
peak the least room it could. The keyboard and menu zoom now put the marker
in the middle of the window, as far as the spectrum's own edges allow; the
wheel still zooms about the pointer, which is where the eye already is.

**The toolbar's Save does what its hover text says (2026-08-13).** The
button promised Ctrl+S and pushed the overwrite-in-place action instead -
Ctrl+Shift+S's. On a file the system would not let the program write over,
the overwrite failed with one status line, read from the bench as the button
doing nothing while the shortcut "worked" - the shortcut's save dialog is
itself what grants the access. Button and key now do the same thing: ask
where, every time.

**One symbol per button (2026-08-13).** With the toolbar set to icons and
words together, the buttons whose labels carry their own symbol - Start,
Stop, to-Buffer - drew the symbol twice: once as the icon, once in the text.
The glyph belongs to the words-only arrangement; beside a drawn icon the
label sheds it.

**Files are suggested with their formats' own capitals (2026-08-13).**
A saved spectrum was offered as `.spe` where MAESTRO writes `.Spe`; the
suggestion and the format picker now spell `.Spe`, `.Chn` and `.Spc` the way
the files' own tools do. Reading was always case-insensitive and stays so.

## 0.2.3-alpha (2026-08-13)

**Naming a peak no longer walks the whole library (2026-08-13).** Every
frame, for every marked region on screen, the display asked the library which
nuclide sits at that energy - and each ask read every line of every nuclide,
allocated a list and sorted it, to keep one name. With an evaluated library
and a background spectrum's two hundred regions that was millions of line
visits per frame, and it is why a marked-up spectrum dragged exactly when a
real library was loaded. The lines are now indexed by energy once and each
ask is a binary search; the frame timing harness puts the worst case (16384
channels, 200 regions, a 27,000-line library) at 0.32 ms/frame where it
measured 2.42 before. The index checks itself against the library every
frame - the library is open data and the editor writes straight through it -
so an edit is seen the frame it is made, never trusted to a stale copy.

**The counting redraw now matches the data it draws (2026-08-13).** While a
count ran, the interface redrew ten times a second against an instrument
mirror that changes twice a second - four identical frames for every real
one, which on a small machine is heat for nothing. Counting now redraws every
250 ms; jobs and the tuning routines keep their 100 ms tick, because they
step forward on every frame.

**The job dialog stopped reading the disk once per frame (2026-08-13).** Open,
it re-listed the job directory and re-read the entire `.JOB` file every frame
- and it is open exactly while a running job holds the frame rate up, so an
SD-card machine re-read its card ten times a second for text that had not
changed. Both now come from disk at most once a second, and immediately when
the directory or the chosen file changes.

**A served instrument answered in two writes with Nagle on (2026-08-13).**
The reply went out, then its newline went out separately - and the client
reads to the newline, so the terminator of a spectrum-sized reply sat behind
a delayed acknowledgement for up to 40 ms of every poll's half-second budget.
One write now carries the line, the server side turns Nagle off as the client
always did, and a write timeout means a client that stops reading can no
longer wedge the single serving slot forever.

**Preset checks run when the numbers can have moved (2026-08-13).** The
acquisition loop evaluated presets on every poll, and the interface polls
every frame - but a remote mirror's clocks and counts change only when a
fetch is integrated, twice a second. An ROI preset walks every marked
channel and an MDA preset runs a whole fit, so those walks re-derived the
same answer from the same numbers sixty times a second. They now run when a
poll actually brought numbers back, and the answer is unchanged - it was
always unchanged, that was the fault.

**Loading a CSV library was quadratic (2026-08-13).** Each row scanned every
nuclide read so far to find its group - tens of millions of name comparisons
over a full evaluated export. A name-to-index map makes the load linear. From
the same pass: the NNDC converter's field splitter reallocated every field of
every row under a comment claiming it did not, and a `.lib` that turns out
to be CSV was decoded from disk twice.

Also caught on the way: the DATA line a served instrument builds twice a
second allocated a String per channel and was then copied twice more to be
"reduced" to the one line it already was; USB spectrum reads zeroed and
copied a scratch buffer per 4 KB chunk; the simulator sorted its events by
timestamp in every mode when only list mode reads timestamps, and cloned its
source list per tick; the peak search allocated a spectrum-sized buffer per
scale, nine per search; Analyse cloned the whole library and the whole
spectrum per click to satisfy a borrow it releases on return; the keyboard
handler cloned the entire input state per frame to read a few dozen
booleans; marking peaks rebuilt and re-sorted the region set per peak; and
an idle session with nothing open deleted an already-deleted snapshot file
every twenty seconds, forever. ARM Linux - the Raspberry Pi, where a bench
in a corner wants this to run - joins the cross-checks CI keeps compiling,
the same road macOS took before it shipped.

**A scan stands the hub down first, and the slow parts of connecting are on
the record (2026-08-13).** Probe and configure are their own processes, and
running one against a live hub is two processes in transaction with one
driver - the contention the hub exists to prevent, felt as scans crawling
through driver timeouts. The hub is parked before any of them runs; open
lanes say the bridge closed, which is the truth, and the next connect starts
a fresh hub. And because the first bench journals could not see where a slow
scan or a slow connect spent its time - all of it ran before anything that
writes fetch lines existed - the journal now times the scan itself, every
phase of connecting including a refusal and its reason, and each step of the
hub opening a detector, road by road.

**Every local detector on Windows shares one bridge process - the hub
(2026-08-13).** The application ran a bridge process per detector entry, and
`serve N` took a lone adapter as the one meant whatever N said - so ORTEC's
configuration, which happily holds several entries for one instrument,
had three processes transacting with one mailbox at once. Their replies
crossed: the spectrum on screen flashed between three interleaved snapshots
twice a second, and the adapter then wedged outright, which is exactly what
the first debug journals from the bench recorded. One process answering one
command before reading the next cannot interleave anything. The dialect
grows a routing mark (`@<n> <command>`); detectors open as they are first
asked for, each through the same USB-then-ORTEC ladder as before, except
that the hub's USB road routes by strict adapter position - no lone-adapter
forgiveness, because the entries must not quietly converge on one instrument
again. An entry that opens nowhere answers `ERR` with the reason, every
time, without poisoning the ones that opened. The libusb road keeps a
process per adapter: adapters there are claimed exclusively, so the
convergence cannot happen.

**`MANTARAY_DEBUG` keeps a written record as well as the overlay
(2026-08-13).** The application journals every fetch (declared count, sum,
clocks, and what the mirror did about it) and every command with its answer to
`mantaray-debug.log`; the bridge writes what the driver actually returned -
asked, returned, masks, sums - to `mantaray-mcb-debug.log`. Both land in the
directory the program was started from, and nothing is opened or written
without the variable set. The overlay can say a frame is late; only the
machine that misbehaves can say what its instrument answered, and now it
writes it down.

**A flapping channel count no longer clears the spectrum from the screen
(2026-08-13).** Reported from the Windows bench the moment acquisition
started: the window flashed empty and full, over and over, as though the
program kept clearing the count. On that road ORTEC's library is asked the
detector's length on every read and truncates the data to what it actually
returned - and a busy instrument mid-acquisition answers short - while the
mirror rebuilt itself from zeros for every length it had not seen before.
A mirror holding counts now believes a new length only when two fetches in a
row agree on it: a one-fetch flap keeps the spectrum on screen and costs that
fetch's channels alone, its clocks still landing, while a genuine
conversion-gain change - which repeats on every fetch - is adopted on the
second. An empty mirror (connection, the fetch after CLEAR) still adopts at
once, because rebuilding zeros as zeros loses nothing. The libusb road never
showed the fault: there the size is asked once and every read is a whole
frame.

**The interface can say what it is waiting for (2026-08-12).** With
`MANTARAY_DEBUG` set in the environment, an overlay reports the adapter,
backend and driver the renderer actually chose, frames drawn in the last
second, the gap before the newest frame, the CPU cost of building it and the
worst such cost in the window, and the surface size - the facts a lag report
needs and Task Manager cannot show. Numbers only, no verdict: only the person
dragging the marker knows whether frames were being asked for. On the bench
that prompted it, one photograph split "the machine is slow" into "the frame
stood waiting on the instrument", which is what the courier below fixed.

**One click of the wheel zooms by one step (2026-08-12).** The wheel took a
full zoom step per *frame* rather than per notch, and egui smooths a single
click across several frames - so one flick of the finger stacked five to ten
steps and the view lunged by 3.6× or more. Wheel zoom is now proportional to
the scroll distance: a notch works out to exactly the keyboard's 1.2× step, a
trackpad glides through the same curve, and a pinch passes its own scale
through instead of being rounded to whole steps. No single frame can move the
view by more than a factor of two, however hard a free-spinning wheel is flung.

**And how far a notch goes is the scheme's to set (2026-08-12).** The style a
scheme carries - tabs or windows, the fill, the glow - now carries the wheel's
pace too: `wheel_zoom`, the share of a keyboard step one notch is worth, from
25% to 400% around a default of parity. Theme & Colours has the slider, the
scheme file has the field, and the curve is the same at every setting - only
its pace changes, so a twitchy wheel can be calmed and a stiff one hurried
without either losing the shape of the zoom.

**A slow instrument no longer freezes the interface (2026-08-12).** The
application refreshed each connected instrument - status and a whole spectrum -
on the thread that draws the frames, so an instrument that took 325 ms to
answer froze the interface for 325 ms, twice a second, for as long as its
window was open. Measured on a bench machine whose ordinary frame costs one
millisecond, and read there as a broken program. Each instrument's transport
now lives with a courier on a thread of its own: the frame asks for a fetch
and collects the finished one later, never waiting on the wire, and commands
still go straight through in order. The command line keeps the old synchronous
road - a script has no frames to hold, and a `WAIT` wants the clocks fresh the
moment it polls.

Caught in review before any of this shipped: a command that raced a fetch
could resurrect what it had just destroyed. The courier runs errands in order,
so a fetch requested before a CLEAR had always arrived by the time CLEAR
answered - carrying the thrown-away spectrum, which the next poll put straight
back, along with a start date reconstructed from the discarded run's clock,
wrong by that whole run and permanent. A command now empties the courier's
slot as it answers, so a poll only ever collects what the instrument said
after the last command. And a courier whose thread has died is reported as
the lost connection it is, rather than a mirror quietly frozen on numbers
that will never change.

Also from the same pass: the sidebar's region list no longer clones its rows
(a Vec of Strings) on every frame it is shown, the library dialog no longer
clones every nuclide's name per frame to paint the list, stepping through
library lines finds the next line in one pass instead of cloning and sorting
the whole library, and a fetch's channel counts are parsed into a buffer that
is kept rather than one allocated fresh twice a second.

## 0.2.2-alpha (2026-08-12)

**A count already running when the window opens keeps its start date
(2026-08-11).** Start an acquisition, close MantaRay, open it again, and the
start time was gone - reported from the bench, and the reason a 5073-second
count came off it with no date at all. Nothing on this road reports a
measurement date: `MIOGetStartTime` belongs to ORTEC's Windows library and has
no counterpart over libusb, so a session that did not watch the run begin had
nothing to ask. The real-time clock is what survives - it advances only while
the run does - so the start is reconstructed as that many seconds ago. Only
while the instrument is still counting, because an idle one holding this
morning's spectrum would otherwise be dated to this minute; and only into a gap,
because a start this session saw for itself is the real one. A run stopped and
resumed reads late by however long it stood paused, which is why this is a
reconstruction and is written down as one.

**A spectrum with no start time no longer claims to have been counted in 1970
(2026-08-11).** `.Spe` was written with `01/01/1970 00:00:00` whenever the start
was not recorded, and that reads back as a measurement made at the Unix epoch -
a date nothing can tell from one that was meant, and one a decay correction will
use without complaint, wrong by decades. The keyword is now left out when there
is no date, which is what the `.Chn` writer has always done by leaving the field
blank; and the old placeholder is read as the absence it stood for, so files
already carrying it stop asserting a date. Found on a 5073-second Cs-137 count
off the bench, which `info` reported as acquired on 1 January 1970.

**The nuclide report's numbers are four significant figures.** The text report
kept three decimal places of its own after the interface's table had moved on,
so a detection limit printed as `2528199.244` and a small activity as `0.003` -
and a report could disagree with the window it came from.

**`docs/nuclide-data.md` no longer promises an export that is not there.** It
said a prebuilt NNDC export and library were attached to the releases; neither
is, on any of them. It now says so, and points at the two sources that do serve
the evaluations.

**The nuclide library dialog shows more than its first few nuclides
(2026-08-11).** The list was built inside a horizontal layout, and a scroll area
lays its contents out the way it was reached - so the names ran off the right
edge in a single line instead of stacking, and everything past the width of the
window was unreachable. With 86 nuclides loaded, most of the library could not
be selected at all.

**A `.json` library opens by being dropped in or named on the command line.**
Only `.lib` and `.csv` were recognised there, so the lossless form of a library -
the one `mantaray library` writes by default - was read as a spectrum instead
and refused with `missing field mantaray`. A `.json` is now offered to the
library reader first and treated as a spectrum only when it holds no nuclides.

**A half life is written in scientific notation when it needs to be.** K-40's
read `1247973344.4968 y`, which claims a precision to the ten-thousandth of a
year that no evaluation states - the digits past the fourth are the length of a
year, not a measurement. Four significant figures throughout, so `1.248e9 y`.

Significant figures, not decimal places, which is a distinction this got wrong
on the first pass: `{:.4}` gave nine figures to an activity of 37 000 Bq - a
microcurie, squarely in the range an operator meets - and two to one of 0.001
Bq, so the wall of digits it was meant to remove survived in the middle of the
range while the weak end lost precision instead. A number is now four figures
wherever it falls, with one changeover into scientific notation rather than a
jump from `99999.9876` to `1e5`. An infinity reads `inf` rather than
`NaNe2147483647`.

**Looking at a half life does not change it.** The field showing one shortens
it for the eye, and egui fills its editor from that shortened text the moment
the field takes focus, then reads the text back when focus leaves - whether or
not anybody typed. So merely clicking in the field rewrote the number: Co-60's
half life lost twelve hours, K-40's ten thousand years, and `Save as...` would
have written the rounding down as though it were evaluated. The field now
recognises its own shortened output and keeps the value that produced it, while
anything actually typed is still read as typed.

**A nuclide is keyed on every strong line it has, not only its strongest.**
Confirmation requires *all* of a nuclide's key lines, so keying Co-60 on 1332
keV alone let any stray peak there pass for cobalt while its 1173 keV twin went
unasked for. Every true gamma at least half as probable as the strongest is now
a key line, up to two - which gives the pairs a spectroscopist would use
(Co-60 1173/1332, Tl-208 583/2614, Ba-133 81/356) without making a nuclide with
a dozen strong lines the hardest one to find. X-rays and the annihilation line
are never keyed on, because neither says which nuclide it came from.

The cap is two rather than three because the half-as-strong rule is relative: a
nuclide whose strongest gamma is faint has a low bar for the next one, so a
spread-out decay scheme collects more required lines than a clean one, which is
backwards - those are exactly the lines a short count loses. Eu-152 is the case
in point, and it is this program's own calibration source: leading at 28.53%,
it admitted a third line at 1408 keV and 20.87%, and requiring that would have
put Eu-152 beyond a NaI detector or a brief count.

**A wedged adapter can be recovered away from Windows (2026-08-11).**
`mantaray-mcb usbfix` existed only on the platform that has ORTEC's driver. Off
it, an adapter whose reply stream had slipped - answering every question with
the answer to the one before, which looks like a working instrument giving
wrong numbers - could be fixed by nothing but the cable. It is now the same
three steps on every platform: drain the queued replies, then, with `--cycle`,
a replug in software, and a plain refusal when neither reaches it. A second 926
on a Mac turned up already in that state, which is how this was found.

Nothing settles automatically, and that is deliberate rather than an omission.
The Linux bench had already established that draining an adapter that was
working stops it working; the same thing on macOS turned three answers out of
three into none, and `docs/ortec-hardware.md` now records the mechanism, which
is specific to libusb. Settling drains before clearing the endpoint halts, not
after, because clearing a halt resets the data toggle that the draining is
liable to have disturbed.

**`mantaray-mcb usbspectrum --out` writes the file it promised.** The flag was
in the usage text on the libusb platforms and did nothing; the spectrum was read
and then dropped. It now saves through the same writer the Windows `dump` uses,
with both clocks in the instrument's own ticks and marked regions carried over
as regions rather than as one per channel. The file records the model the way
`probe` prints it - `0926-001`, not the raw `$F0926-001` record - and `--out`
with no file after it says so before the adapter is opened rather than reading
four thousand channels and then exiting quietly as though it had written them.

**The 926 bench test can be run the way it says to run it.** Its three tests
each claim the adapter exclusively and cargo runs tests in parallel, so at most
one could pass and the others failed reporting a busy interface - which reads as
broken hardware. They now take turns on the same lock `bridge_hardware.rs` uses.

**The command-line tests stop pulling their fixture out from under each other.**
Four of them share one library file and rewrote it on every call, so cargo
running them in parallel meant one test truncating the file while another read
it - which surfaced as `peaks` exiting with "missing library rows" and an empty
stdout, reading as a broken peak search rather than as a broken fixture. It
failed that way on CI on 2026-08-12. The fixture is now written once per test
binary.

## 0.2.1-alpha (2026-08-08)

**macOS ships (2026-08-07).** It was held back on the rule that an untested
binary for a platform nobody has used is a promise this project cannot keep.
That has now been paid: an ORTEC 926 was driven from a Mac through a DPM-USB
adapter over libusb, with no vendor driver and none of ORTEC's user-mode
software - enumerated, identified, read out whole in about 90 ms, counted
through a live preset it stopped itself on, and carried into the desktop
application. There is no permission step; macOS needs no equivalent of the
Linux udev rule, because nothing claims a vendor-specific interface. The
archive is for Apple silicon and is unsigned, so the first run needs the
quarantine attribute removed. Intel macs are still not built, for exactly the
reason macOS was not.

**A calibration can be recalled from the file that holds one (2026-08-07).**
MAESTRO saves an energy and peak-shape calibration to a `.Clb` of its own so it
can be put onto a spectrum taken later, and `Calibrate / Recall Calibration`
read spectra only - so the one file whose entire purpose is this was the one
thing it would not take. The format was decoded from samples against `.Spe`
files the same detectors saved on the same days; `docs/formats.md` records what
is known and, at more length, what is not. Writing `.Clb` is not implemented,
because the rest of the file cannot be reproduced faithfully.

Recalling is now the same operation wherever it is reached from. A `.JOB` gets
`.Clb` files too, and the guards that used to exist only in the menu now hold
in automation as well: a file carrying no calibration is refused rather than
copied over a good one, and a peak shape already in hand outlives a file that
records none - including a `.Clb` whose shape terms are all zero, which is the
absence of a shape rather than a shape of zero width. Recalling with no
spectrum open says so instead of reporting the coefficients it applied to
nothing.

## 0.2.0-alpha (2026-08-07)

**The project is now MantaRay.** Gamma rays, manta rays. The crates and binaries
are `mantaray-*` and the environment variables are `MANTARAY_*`; the former name
stays in the history, where it is a record rather than a claim. Settings, the
tuned palette, the recent files and any snapshot of unfinished work are carried
across on first run, under both the old directory and the old key inside it -
without which a renamed build would have started as though it had never run.

**The look is the operator's.** A colour scheme is no longer only a palette: it
carries how the program draws, and it travels as a small JSON file that can be
sent to somebody. Seven schemes ship, including Conductor, whose colours were
sampled from a screenshot of the software these instruments have traditionally
been driven with rather than remembered. Every colour is editable and the
contrast of each against the plot is reported as it is edited.

**Spectra are arranged in tabs**, one filling the working area, with any of them
pulled out into a window when two must be watched at once. **Workspaces** decide
what the sidebar shows for the job in hand - the clock and the presets while a
count runs, the regions and the nuclide lookup afterwards.

**A nuclide can be named.** Type `Cs-137`, `137Cs`, `cs137` or `Cs 137` and its
lines are drawn over the spectrum, each labelled with its emission probability.
When it is not found, the reason distinguishes no library loaded from not in
this library from not a nuclide name.

The toolbar can carry drawn icons instead of words. They are drawn rather than
written because a character that looks right while writing the code arrives on
somebody else's machine as an empty box, which has happened here twice.

Also in this release, the nuclide-library work below, and a busy adapter now
told apart from one there is no permission for - errno 16 rather than 13, where
no udev rule has ever helped.

Binaries for Linux and Windows are attached, with a SHA-256 for each. macOS is
built and tested by CI and deliberately not released: nothing has run there
against an instrument.

## Unreleased

**No nuclide library ships any more, and there is a way to build one
(2026-08-07).** Line energies and emission probabilities are somebody's
evaluation: they have authors, a publication cut-off and stated uncertainties,
and a measurement computed from them can only be defended if you can say which
evaluation you used. What used to be built in was a table keyed in by hand
carrying none of that - close enough to the accepted values to look
authoritative, and answering to nobody.

It is gone from every road into the program. A fresh session, and the command
line with no `--library`, now start with nothing, and the analysis table, the
isotope markers and the nuclide reports say so instead of showing an empty
result that reads like a finding; the command line refuses rather than
reporting nothing found. The few values kept for the test suite are named
`sample_for_tests` and documented as a fixture, not a library.

In its place, `mantaray library --nndc export.csv -o nuclides.json` turns the
National Nuclear Data Center's radiation export - the ENSDF evaluations, served
through NuDat - into a library, keeping every number as evaluated. It selects
photons and drops betas, conversion electrons and alphas; tells gammas, X-rays
and the annihilation line apart; names metastable states; keeps the strongest of
duplicate lines; and records what it converted and when, so a report can cite
it. Run against the full export it reads 242,295 rows into 2,043 nuclides and
27,015 lines, and the values land where they should: Cs-137 at 661.657 keV and
85.1%, Co-60's pair at 99.85 and 99.98%, Eu-152's ladder, K-40 at 1460.82.

Rows are split on the commas that separate fields rather than on every comma.
Some fields are quoted and hold their own - the spin-parity column carries
values like `"(0-,1-)"`, which is the ordinary way to write an undecided
assignment - and a comma inside those quotes shifted every column after it.
The four columns the reader needs sit at the end of a forty-two column row, so
all four moved together and the row quietly stopped looking like a gamma. It
was never wrong, only short: 550 evaluated lines and 31 nuclides went missing
without a word, among them Bi-218 from the radon chain, Rh-102 and Pm-148m.

See [docs/nuclide-data.md](docs/nuclide-data.md) for where to get an export and
who to credit - the NNDC and the IAEA for the evaluations, carsus for retrieving
them, and Dani Solakian and Berkeley RadWatch for the compiled database this
converter is built around, used with permission.

**Work in progress survives a crash, and a count is watched while it runs
(2026-08-06).**

*Nothing used to persist the working session.* Instrument data survives
anything, because the MCB owns its own memory - but a recalled file, a
spectrum that had been smoothed or stripped, the buffer recovered from a
Clear, calibration points half entered, all existed only in the process, and
none of it is on disk until somebody saves. Nobody saves before a crash. A
snapshot is now written every twenty seconds and deleted on a clean exit, so
a snapshot found at start-up *is* the evidence that the last run did not
finish; what it holds is offered back, listing each window and its counts,
rather than reappearing unbidden - windows opening by themselves is how
somebody ends up working on the wrong data. Detector windows are deliberately
not snapshotted: they are a view onto instrument memory, and restoring a
stale copy would show counts the instrument no longer has. The snapshot is
written to a neighbouring file and renamed over the old one, so a crash
*during* a write cannot destroy the thing that survives crashes.

*A count now leaves a record of how it behaved.* The readout says what the
rate is; it cannot say whether it has been that all along, and the things
that quietly ruin a measurement - a source that shifted, a detector warming
up, a shield left open - are all changes over time. The sidebar draws the
count rate across the acquisition with dead time faint underneath, and states
the drift between the first and last fifth of the run, coloured only once it
is past what counting statistics explain. The rate plotted is the interval
rate rather than the average since the start, because an average hides
exactly what this is for. A long count is thinned rather than truncated, so
the beginning - which a drift is measured against - is never the part thrown
away, and a Clear starts the log over rather than drawing a line across the
discontinuity. This is separate from the QA control charts, which compare
check-source measurements across days; this is one acquisition, from inside.

**Clearing lets the next count start again (2026-08-06).** Found by running
the bench check against the 926: `CLEAR` resets the instrument's clocks, but
the mirror kept the old ones until the next poll - and the start guard added
with the preset work reads them. With a live preset held, Clear and then Start
was refused as "the Live time preset is already reached - clear the spectrum,
or change the preset", which told the operator to do the very thing they had
just done; a job's `CLEAR` / `START` did not start at all. The mirror's clocks
now go with the instrument's, as the simulator's always have. The whole
sequence is pinned on the hardware now: count out a one-second preset, watch
the instrument stop itself at LT=1.00, and see the next `START` refused by
name - the first end-to-end proof of that guard on the instrument it was
written for.

Also here: a doc comment had been separated from what it documents by a
function inserted between them, leaving `logarithmic_ceiling` undocumented and
its text attached to `decade_ceiling`.

**Five things found by daily-driving it (2026-08-06).** All raised by the
person using the program against a real detector, and all small enough to be
felt every session.

- *Ctrl+S asks where to save, every time.* It used to overwrite whatever the
  spectrum was last saved to, with no prompt - a reflex keystroke away from
  replacing a measurement that cannot be taken again. Ctrl+S is now Save As,
  the File menu leads with it, and overwriting in place has moved to
  Ctrl+Shift+S, where the menu entry says which file it will replace.
- *The count rate is shown under the total counts*, over live time rather
  than real, with the precision the size of the number deserves: a background
  at a fraction of a count per second and a check source at thousands both
  have to read sensibly from the same field.
- *A logarithmic axis can top out a whole decade above the peak*, keeping the
  figure it leads with: 300 counts tops out at 3000, 1000 at 10,000. The peak
  is then read against a number drawn below it rather than against the
  ceiling. Display / "Log to the next decade", off by default so the existing
  tighter scaling stays. Anything after the leading digit rounds it up - 347
  tops out at 4000 - and because the axis can now stop between decades, the
  number it stops at is labelled in its own right.
- *Peak Info fits a dragged selection.* Drag a span, right-click, Peak Info,
  and the fit is over exactly what was dragged - with no region marked.
  Measuring a peak no longer edits the spectrum's regions as a side effect
  and leaves them to be tidied up afterwards.
- *Clear ROI over a selection clears every region in it*, in one undo step,
  rather than only the one under the marker. A region straddling the edge of
  the selection goes with the rest: it was pointed at, and half a region left
  behind is a stranger result than none.

**The bench check, run (2026-08-06).** The preset work below was written
against doubles and marked as not yet run on the 926; it has been now, and it
holds: `probe` finds the adapter, `SHOW_PRESETS` read back the very 300-second
live preset the instrument was found holding, `SET_PRESET_LIVE 2` round-tripped
and stopped the count at exactly LT=2.00, and `START` against that satisfied
preset was accepted with the clocks frozen - the silent refusal, reproduced.
Pinning by `--device` opens the named adapter and refuses a serial that is not
there; a detector opened twice learns its serial and then matches it. The bench
check is kept as `crates/mantaray-device/tests/bench_926.rs`, ignored by default
because it needs the instrument. Three things it found:

- *Dead time could read below zero.* The two clocks are separate commands, so
  they are sampled a round trip apart, and the one read second has run on: at
  a low count rate the bench 926 reported `RT=1.00 LT=1.02 DT=-2.00%`. Real
  time is now read second, which puts the skew in the direction the arithmetic
  survives, and the result is clamped - a negative dead time is a sampling
  artefact, not a measurement.
- *A helper that refuses to start said why to nobody.* Its reason goes to
  standard error, which a window with no console behind it never shows, so the
  operator saw only "the bridge stopped". Standard error is now relayed line by
  line as before *and* kept, so the failure is reported as "the bridge stopped:
  no adapter with "08134079" in its serial number".
- Two doc comments had been separated from what they document by code inserted
  between them, leaving `spawn_program` and `close_window` undocumented and
  their text attached to a test module and to `remember_serial`.

**A detector can be named, and cannot be confused with another one
(2026-08-06).** Three related things, all about knowing which instrument you
are talking to.

*Detectors can be renamed.* The Detector List's name column is an edit box:
type over it and the new name reaches the saved pick list, the open
instrument, its window title, and the detector fields of every spectrum saved
from it afterwards. A name that means something to whoever reads the file next
year - "Bench HPGe" - beats the model and serial the scan happened to print.

*A local instrument is pinned to its adapter, not to a position on the bus.*
Away from ORTEC's configured detector numbers, `serve N` means the Nth adapter
the bus enumerates, which is a position and not an instrument: plug in a
second adapter, or replug the one there, and the same entry can lead somewhere
else. An entry now remembers what the instrument called itself and hands that
to the helper as `--device`, which both the Windows and the libusb bridge
already understood. The one case where that would be wrong is handled: through
ORTEC's library an instrument reports the detector number that selected it,
and a number is not an adapter serial, so an identity that is only the number
already used is not treated as a pin.

*And the identity is checked on every open.* Whatever the instrument reports
in its configuration reply is compared against what the entry remembers -
learned on the first successful open - and a mismatch refuses the connection
by name ("this is instrument \"11217584\", not \"08134079\" - the adapters may
have been swapped or replugged") instead of quietly opening the wrong detector
and labelling its spectra with the wrong name. An instrument that reports no
serial is nothing to check against rather than evidence of a mix-up, so those
still open.

**A preset the instrument is already holding is visible, and cannot silently
refuse a count (2026-08-06).** Found on the bench: a 926 was still holding a
300-second live preset from an earlier session, with its live clock stopped
at exactly that value. mantaray could only ever *write* presets - it opened
with an empty Presets tab, because nothing ever asked the instrument what it
was holding - and then `START` was accepted and the instrument did not count,
because the preset was already satisfied. The instrument says nothing about
either: a valid `START` and an ignored one look identical on the wire, and no
reply carries the preset registers unless they are asked for.

Both halves are closed. The bridge answers a new `SHOW_PRESETS`, reading all
four registers back (time presets in ticks, reported in seconds; counts as
counts), and a served simulator answers it the same way, so an instrument and
a simulator behave alike. Connecting asks the question and shows what comes
back, so the Presets tab reflects the instrument rather than only this
session's edits; an older bridge that does not know the verb still connects,
with no presets shown, exactly as before. And starting an acquisition whose
real-time, live-time or ROI preset is already reached is now refused by name -
"the Live time preset is already reached - clear the spectrum, or change the
preset" - instead of appearing to work. Uncertainty and MDA presets are not
part of that check: they are host-side, and `advance` stops and reports them
on the next poll rather than the instrument quietly declining.

**Question and answer stay together on the bridge (2026-08-06).** Reviewing
the work below found that giving the bridge transport a ten-second timeout
had introduced a way for it to answer the wrong question. A timed-out
exchange left its late reply in the channel, so the next command read that
reply as its own and every reading afterwards was the previous one's - for as
long as the connection lasted, and silently: a status poll that lands on a
`DATA` line parses as no counts and not counting, which is exactly what an
idle instrument looks like. A stall that long needs nothing exotic; a
suspended laptop is past ten seconds on its own. The transport now counts the
replies still owed by commands it gave up on and discards that many before
believing an answer, so it either recovers alignment or keeps failing - never
returns a reply belonging to something else. Alongside it, the bridge reduces
each reply to the one line the protocol allows: passthrough relays the
instrument's own words now, a reply is only cut at a carriage return or a
NUL, and a lone line feed in one would have arrived as two answers and put
every later question with the wrong one.

**The audit's open list, worked through (2026-08-06).** Everything the
2026-08-04 audit left in TODO.md is fixed except what needs the bench or a
product decision; TODO.md now holds only those. In severity order:

- *Global shortcuts stand down while typing.* Every unmodified key binding -
  Delete, Insert, Home/End, the arrows, `+ - / =`, `A`, `5` - now yields to
  a focused text field, as Escape always did. Pressing Delete to fix a typo
  in the Calibrate dialog's energy box no longer silently clears the region
  under the marker, and typing "511" into an energy box no longer recentres
  the view.
- *Job `WAIT`s wait, and job presets reach the wire.* On a real instrument,
  `WAIT n` now spends n wall-clock seconds and `WAIT` polls a few times a
  second until the preset stops the count - previously both returned in
  milliseconds, so the manual's own `START / WAIT 300 / STOP` pattern saved
  a near-empty spectrum; in the desktop application the job now parks
  between frames, so the interface stays alive for the whole count instead
  of freezing while hammering the wire. Job `SET_PRESET_*` goes through
  `set_presets` and reaches the instrument, where before it wrote only a
  client-side mirror that died with the process. The simulator keeps its
  faster-than-real-time fast-forward throughout.
- *A quoted job argument followed by a space.* `LOCK "pw" "owner"` parsed as
  password `pwowner` with an empty owner - a password the operator never
  typed, on a detector now locked with it. Whitespace now ends a quoted
  argument exactly as a comma does.
- *One corrupt reply cannot take down the client.* A malformed `SHOW_DATA`
  line - a huge declared count, a word that is not a number, fewer words
  than declared - is now a reported error that leaves the last good
  spectrum standing. Previously a huge count aborted the whole process
  inside an allocation, and a garbled word quietly became zero counts.
- *File readers refuse corrupt lengths instead of allocating them.* A
  `.Spe` whose `$DATA:` range declares nine hundred million channels, a
  list-mode file whose channel-count field was rewritten, an N42 whose
  runs of zeros are individually modest but cumulatively gigabytes, and a
  crafted `.Lib` whose 65535 nuclides all share one 65535-peak chain are
  all errors now, not aborts or hangs. The N42 element walk is a loop
  rather than recursion, so a crafted file cannot overflow the stack.
- *The Windows IOCTL path is sound on 64-bit.* OVERLAPPED was modelled as
  five pointer-sized words, which on x86_64 leaves the real `hEvent` null -
  the kernel signalled the file handle instead, ambiguous the moment two
  requests ever overlap. It is now a proper `#[repr(C)]` struct, correct on
  both widths. When a timed-out request cannot be withdrawn from the
  driver, the transfer buffers - now heap-owned for exactly this reason -
  are leaked rather than freed under a kernel that may still write through
  them, and the device is retired until reopened. Registry enumeration
  skips an unreadable key instead of hiding every device after it.
- *The bridge cannot hang the application.* A wedged helper process used to
  block the pipe read forever; answers now arrive through a reader thread
  with a ten-second bound, and closing a wedged bridge kills it after two
  seconds rather than waiting indefinitely.
- *Dialog state ends with its dialog.* The strip dialog no longer reports
  "stripped" over a failure; removing a calibration row no longer smears
  the deleted row's typing onto the row below, and half-typed calibration
  edits die when the dialog closes; the Report viewer and the Exit and
  Unsaved-changes questions stay above a maximized plot; the regions
  sidebar refreshes when the calibration, library or settings change, not
  only when counts do.
- *Windows are addressed by identity.* Closing a background window no
  longer silently retargets commands at whatever window happens to be last;
  the Window menu and double-click focus by window id, so a list that
  shifted mid-frame cannot activate the wrong one. A view that showed the
  whole spectrum keeps showing all of it when the conversion gain rises.
- *The first frame shows before the instrument scan runs*, so launching
  with no detector configured shows a window immediately instead of
  appearing to hang for the length of a synchronous probe.
- *The serve dialect answers with the instrument's own words.* A `$`- or
  `_`-spelled command passed through the bridge now relays the reply
  (SEND_MESSAGE can finally see its answer); record parsing checks the
  record letter, so a desynchronised version record can no longer read as a
  plausible clock; a UMCBI model name with a space no longer shifts the
  configuration line's fields.
- *Simulator edges.* Storing on a locked instrument no longer half-happens;
  clearing list mode keeps the file's description and calibration;
  `SEND_MESSAGE("SET_PRESET_…")` respects the same no-change-while-counting
  rule as the dialog; a served instrument's pole-zero completes while idle;
  the TCP server survives a poisoned lock and drops a silent client after
  a minute instead of blocking every later connection.
- *Job engine edges.* The line number reported after a job ends is the last
  command's, not the command count; the million-step runaway guard says
  what it is; the chaos corpus's bare `BEEP` (no such form - the manual
  gives `BEEP <freq>,<duration>`, `BEEP ID` and `BEEP "String"`) is fixed
  and the corpus now asserts it parses. UMCBI loading no longer leaks a
  module reference per failed attempt, unpins its DLL directory on failure,
  and resolves `mcbloc32.dll` to the copy already in the process.

**Linux drives real hardware (2026-08-05).** The libusb path met an ORTEC 926
for the first time - it had compiled for months and never moved a byte - and
every recorded wire-format assumption held on first contact: the `$F`/`$G`
records, the tick arithmetic, and a whole 4096-channel spectrum with its ROI
bits in about a fifth of a second. What changed to get there:

- *`mantaray-mcb serve` exists away from Windows.* The dialect-translation
  layer (`Session`) is compiled on every platform and a new `ViaDirect`
  backend carries it over libusb, so the desktop application drives a local
  instrument on Linux exactly as it does on Windows - Scan, Open all, and a
  live detector window, with no vendor software of any kind.
- *`probe` and `configure` answer on Linux* in the same block shape the
  Windows bridge prints, so the application's scan parses both without caring
  which platform answered. `usb` keeps the plain serial listing.
- *The application looks for `mantaray-mcb`* (no `.exe`) beside itself away
  from Windows.
- *The translation layer is now tested everywhere*: a bench double answering
  as the real 926 answered pins the status clocks, preset tick conversion,
  data reads and refusals on every platform, not only where the hardware is.

Opening the adapter needs a one-line udev rule (see the README); the first
failure was `errno 13` and nothing else, exactly as predicted in
[docs/ortec-hardware.md](docs/ortec-hardware.md). macOS still type-checks and
nothing more.

**File dialogs hid the very files they were for.** On Linux, Recall with the
`.Spe` filter showed nothing: ORTEC names its files capitalised - `.Spe`,
`.Chn`, `.Spc` - and GTK and the XDG portal match filter patterns
case-sensitively where Windows does not, so a lowercase-only filter hid every
real instrument file. Filters now carry each practical spelling, at the one
place the platform is spoken to, so every dialog in the application is fixed
at once.

Presets could not be typed in at all: ticking a preset's box un-ticked it on
the next frame. The Presets tab re-read the instrument's own presets every
frame, so a half-finished edit - ticked, but not yet applied - was erased
before the number could be typed. The edit now lives in the dialog until
Apply sends it (or Clear discards it), and switching detectors seeds it
afresh so one instrument's presets are never typed into another's.

Testing MCB Properties against the bench found the count presets never
arrived: the bridge passed `SET_PRESET_COUNT`/`SET_PRESET_INTEG` through
untranslated, and the 926 ignores an unknown verb with the same empty reply
it gives a valid one - so the bridge answered OK while setting nothing. The
real verbs, established on the bench by writing values and reading them back
exactly, are `SET_PEAK_PRESET` and `SET_INTEGRAL_PRESET`, in counts;
`SET_PRESET_CLEAR` now zeroes all four preset registers. The time presets
were verified whole: a 10-second live preset set through mantaray's dialect
stopped the instrument by itself at exactly LT=10.00 s. Uncertainty and MDA
presets are host-side calculations no instrument carries, and this was
recorded here as not working on a remote instrument at all; that was wrong.
They are evaluated from the mirrored spectrum on every frame and STOP is sent
when one is satisfied. What is really different from a time preset is where
the stop lives: the instrument's own registers hold a time preset and stop it
whether or not anything is watching, while these hold only for as long as
mantaray is running.

The first double-click on the bench's own Cs-137 peak found another gap:
without a region or a shape calibration, Peak Info guessed the peak to be
eight channels wide and fitted a sliver of anything broader - and a NaI(Tl)
line can be a hundred channels wide. The peak search now measures the peak
under the marker from the counts themselves, and the fit takes the whole
peak, centred where the peak is. The eight-channel guess remains only as the
last resort when no peak can be found at all.

The first spectrum saved off the bench found a gap that was never
Linux-specific: a spectrum from **any** remote instrument - a network MCB, or
either bridge - saved with `DET# 0`, no detector name and a measurement date
of 01/01/1970, because the remote mirror never carried them. The detector's
number and name from the pick list now travel with its spectrum, the moment
START is accepted is recorded as the measurement date (a resumed count keeps
its original date; Clear resets it, as the simulator always did), and none of
it is lost if the instrument's conversion gain changes between polls.

**Audit fixes (2026-08-04).** A five-pass review of the whole workspace; what
it found and what was fixed:

- *Energy and shape calibration fitting.* A duplicated calibration point no
  longer breaks (or silently skews) the fit - repeated channels average, and
  the fit is least-squares over every point. The quadratic fit is solved in a
  centred, scaled coordinate, so 16k-channel spectra no longer shred the
  curvature coefficient. Auto-calibration refits exactly-three matches as the
  line the search validated instead of an exact parabola through
  tolerance-sized misses, and one reference line can no longer be claimed by
  two peaks (an unresolved doublet could previously stand in for two of the
  required three agreeing lines).
- *The uncertainty preset* now stops on the 1-sigma uncertainty of the **net
  peak area**, as the manual defines it ("calculated in the same manner as
  for the Peak Info command") - not on gross counts, which on a
  continuum-dominated region would stop acquisition far too early.
- *Quantitative analysis* reports why a requested thing could not be done
  instead of silently omitting it: a decay correction with no recorded start
  time, and each absent-nuclide MDA row that could not be built, now land in
  the report's notes.
- *QA control charts* flag a not-a-number measurement as Action instead of
  letting it sail through as in-control.
- *DPM protocol (in-house USB).* Partial frame sends, empty write
  acknowledgements and replies whose length word promises more than arrived
  are now errors instead of silent desynchronisation; `read_all` refuses
  reads that would wrap the sixteen-bit address space and return duplicated
  memory. The libusb backend uses cancel-on-timeout transfers throughout, so
  a timed-out transfer can no longer hand its stale completion to the next
  request as if it were the answer.
- *Report output.* CSV fields are quoted when they need it (a comma in a
  nuclide name no longer shifts every later column), and analysis notes are
  printed with the nuclide report.
- Half-life display no longer renders whole values with a trailing dot
  ("1248000000. y"); FW(1/x)M clamps its `x` to the documented 2..=99 on
  input as well as in the settings dialog.

The findings not yet fixed - with file, line, failure scenario and suggested
fix for each - are recorded in [TODO.md](TODO.md).

## 0.1.0

The first release. Everything in the MAESTRO v7 (A65-B32) manual is
implemented or deliberately improved - [docs/maestro-parity.md](docs/maestro-parity.md)
is the section-by-section accounting.

**The workbench.** Acquisition against a physics-based simulator or a network
instrument; PHA, list-mode and zero-dead-time modes; presets on time, counts,
uncertainty and MDA; two-view spectrum windows with marker, regions,
comparison traces, isotope markers and an in-plot peak-information card;
energy, peak-shape and efficiency calibration; peak search, peak info, sum,
smooth, strip; ROI and nuclide reports; the complete `.JOB` automation
language including `RUN`/`WAIT`, `LOOP SPECTRA`/`VIEW` and `ZOOM`.

**File formats.** `.Chn`, `.Spc`, `.Spe`, `.Roi`, TRANSLT-style ASCII, a
lossless native JSON and a time-sliceable list-mode container - with files
recognised by their contents when the name says nothing, and readers that
survive arbitrary corruption. Validated against genuine MAESTRO Pro spectra.

**Beyond the manual.** Undo that recovers cleared instrument data into a
buffer window; automatic energy calibration from a known source's peaks;
list-mode replay; SVG plot export and browser-based printing with the plot
and reports on one page; QA control charts; a multi-detector dashboard,
alarms and a batch queue; four colour schemes held to measured contrast
rules; window tiling; a keyboard reference on F1; settings that survive a
restart; and a headless test suite that drives the real application - about
550 tests in all, including a monkey that hammers the session with hostile
actions and has already caught a crash.

**Care in the corners.** The analysis table links every nuclide to its
strongest line in the spectrum; detection limits stay meaningful without an
efficiency curve (count-rate units, like the rest of the table); an idle
empty detector says what to do next; dragging a file over the window
announces "drop to open"; the title-bar close button asks the same
unsaved-work question File/Exit does; and the toolbar, header and sidebar
degrade gracefully at narrow widths instead of clipping.

**Real hardware.** Instruments are reached through a transport carrying the
ASCII `SET_`/`SHOW_` dialect. TCP is built in and proven against a served
simulator (`mantaray serve`).

Beyond that, MantaRay talks to a real ORTEC 926 over USB **using none of ORTEC's
user-mode software** - no `Mcbcio32.dll`, no `mcbloc32.dll`, no
`DpmUsbAddIn.dll`, only the kernel driver. Commands, clocks, configuration,
gain, mode, integrals, the dual-port memory and whole spectra all work, and the
readout was checked channel-for-channel against ORTEC's own library on the same
instrument: 8192 of 8192 identical, clocks matching to the millisecond, and the
totals agreeing with the instrument's own arithmetic. `mantaray-mcb` is the
32-bit bridge, and it can also drive ORTEC's libraries where they are
installed. The wire format is written down in
[docs/ortec-hardware.md](docs/ortec-hardware.md).

**ORTEC's own files.** Binary `.Lib` nuclide libraries are read as GammaVision
writes them, chain-walked rather than trusted in file order, and every library
that ships with MAESTRO loads.
