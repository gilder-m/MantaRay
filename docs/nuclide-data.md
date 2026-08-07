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
read 242295 rows, kept 27402 lines: 2043 nuclides, 27015 lines written to nuclides.json
```

Then load `nuclides.json` like any other library.

`--min-intensity` sets how faint a line may be and still be carried. The
default of 1% keeps what a detector can actually see; `--min-intensity 0` keeps
every evaluated emission, which makes a much larger library that identifies
nothing more.

### What the converter does, and does not, do

It selects photons — the `g` rows — and drops betas, conversion electrons and
alphas, none of which make a peak in a gamma spectrum. It tells gammas, X-rays
and the annihilation line apart from the export's own subtype column. It names
nuclides `Cs-137`, with an `m` for a metastable state. Where the same line
appears under more than one decay branch it keeps the strongest, and it marks
each nuclide's strongest true gamma as the key line.

**It invents nothing.** Every energy, emission probability and half-life in the
result is the evaluated value, unchanged.

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
