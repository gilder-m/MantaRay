# Where the nuclide data comes from

**No nuclide library is shipped with this project, and that is deliberate.**

Line energies and emission probabilities are somebody's evaluation. They have
authors, a publication cut-off and stated uncertainties, and a measurement
computed from them can only be defended if you can say which evaluation you
used. A table typed in by hand, carrying none of that, is worse than no table
at all — it looks authoritative and answers to nobody.

So there are two honest ways to get one, and both are here.

## Bring your own

ORTEC binary `.Lib` files are read as GammaVision writes them, chain-walked
rather than trusted in file order. JSON and CSV libraries work too.

**Analyse → Library file...**, or `--library` on the command line.

Until a library is loaded, the analysis table, the isotope markers and the
nuclide reports say so, rather than showing an empty result that could be
mistaken for a finding.

## Build one from an evaluated source

The National Nuclear Data Center at Brookhaven publishes the ENSDF evaluations
through NuDat as a flat radiation table: one row per emission, per decay
branch, per nuclide, with energies, emission probabilities, uncertainties and
half-lives. That export converts directly into a library.

```sh
# The export is normally distributed gzipped.
gunzip -c gamma_db_nndc.csv.gz > nndc.csv

ortseam library --nndc nndc.csv -o nuclides.json
```

which prints what it did:

```
National Nuclear Data Center (NuDat/ENSDF) radiation export, converted 2026-08-07;
lines at or above 1% emission probability
read 242295 rows, kept 27402 lines: 2163 nuclides, 27262 lines written to nuclides.json
```

Then load `nuclides.json` like any other library.

`--min-intensity` sets how faint a line may be and still be carried. The
default of 1% keeps what a detector can actually see; `--min-intensity 0` keeps
every evaluated emission, which makes a much larger library that identifies
nothing more.

### What the converter does, and does not, do

It selects photons — the `g` rows — and drops betas, conversion electrons and
alphas, none of which make a peak in a gamma spectrum. It tells gammas, X-rays
and the annihilation line apart from the export's own subtype column. Where the
same line appears under more than one decay branch it keeps the strongest, and
it marks each nuclide's strongest true gamma as the key line.

It keeps each **decaying state** separate, which matters more than it sounds.
The export gives one row per emission per *parent state*, and a nuclide's
isomer has its own half-life and its own emission probability for a line the
two states share — Sc-56 emits 1128.7 keV at 18% from its ground state and at
30% from the state above it. Merging them takes one half-life and the larger
intensity, and an understated yield **overstates** the activity computed from
it. States are told apart by the level they decay from, so the result is
`Cs-137`, `Ba-137m`, `Ir-194m2`: the bare name for a state the export places at
zero, then `m`, `m2`, `m3` upward in energy. The export's `Metastable` column is
not read at all — it is set on every row of a nuclide that *has* an isomer,
rather than on the isomer's own rows, so it cannot distinguish them.

A nuclide the export never places at zero appears only as `m` — the evaluation
has not determined which of its states is the ground one, and this will not
guess.

**It invents nothing.** Every energy, emission probability and half-life in the
result is the evaluated value, unchanged. Where the export marks a half-life
undetermined, the result says undetermined rather than substituting a number.

### Getting the export

A prebuilt export and library are attached to the
[releases](https://github.com/gilder-m/ORTSEAM/releases) so you do not have to
assemble one to get started.

To build the export yourself, the compiled NNDC gamma database used here comes
from the Berkeley RadWatch [`spectral-analysis`](https://github.com/RadWatch)
project, where it is produced with
[carsus](https://github.com/tardis-sn/carsus) — the TARDIS project's tool for
retrieving and caching NNDC tables. The
[IAEA Live Chart of Nuclides API](https://www-nds.iaea.org/relnsd/vcharthtml/api_v0_guide.html)
serves the same ENSDF evaluations one nuclide at a time and is a good check
against a single value.

## Credit and terms

- **National Nuclear Data Center**, Brookhaven National Laboratory — NuDat, over
  the ENSDF evaluations. The data.
- **IAEA Nuclear Data Services** — the Live Chart of Nuclides. Downloading and
  storing their data locally is expressly permitted; attribution to the IAEA is
  required.
- **[carsus](https://github.com/tardis-sn/carsus)**, TARDIS project — retrieval
  and caching.
- **Dani Solakian** and Berkeley **[RadWatch](https://github.com/RadWatch)** —
  the compiled gamma database this converter is built around, used with
  permission.

If you publish a result computed with a library built this way, cite the
evaluation, not this program. The provenance line the converter prints is there
to be copied.
