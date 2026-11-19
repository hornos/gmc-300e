# Reference documentation

This folder is where `scripts/fetch-assets.sh` places the freely downloadable
GQ reference documents. They are **not committed** to this repository because
they are copyrighted by GQ Electronics LLC; the paths are listed in
`../.gitignore`. Run the fetch script after cloning if you want local copies.

| Path (local only) | What it is |
| --- | --- |
| `manual/GMC-300E-Plus-User-Guide.pdf` | Manufacturer user guide, Rev 2.13 (June 2022). Covers the GMC-300E Plus / GMC-300S family, which includes the tested `GMC-300Re 4.70`. |
| `manual/GMC-Quick-Start-Guide.pdf` | Short GQ GMC series quick-start guide, V1.9. |
| `protocol/GQ-RFC1201.txt` | Official GQ serial protocol specification, Ver 1.40. Note that this controller is tuned against the real device as well; see `../README.md` and `../skill/references/protocol.md` for corrections and caveats. |

Upstream links, authors and licences are recorded in `../CREDITS.md`. These
documents are third-party material and are **not** under this repository's MIT
licence.

## Refresh the files

From the repository root:

```sh
./scripts/fetch-assets.sh          # download all files and verify checksums
./scripts/fetch-assets.sh --check  # verify the files that are present locally
```

`scripts/checksums.sha256` pins the exact bytes expected from upstream, so a
changed upstream file is noticed. `--check` reports the uncommitted GQ
documents as skipped when they are not present. If an upstream document is
updated, edit the URL/checksums deliberately rather than blindly overwriting.
