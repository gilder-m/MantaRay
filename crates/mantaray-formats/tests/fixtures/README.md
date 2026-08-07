# Format fixtures

Drop real instrument files here to test the readers against genuine data:

- `*.chn` - ORTEC integer spectra
- `*.spe` - IAEA ASCII spectra
- `*.spc` - ORTEC binary spectra
- `*.roi` - region tables
- `*.lis` - list-mode data

`cargo test -p mantaray-formats` then loads every file in this directory, checks
the readers accept it and round-trips it through the native format. An empty
directory simply skips those tests, so a clean checkout stays green.

Files in this directory are ignored by git (see `.gitignore`) so that sample
data with real measurements is never committed by accident.
