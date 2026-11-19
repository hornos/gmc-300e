# Offline documentation

This folder holds freely available reference material for the GQ GMC-300E
family, so the project is useful without a network connection.

| Path | What it is |
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
./scripts/fetch-assets.sh          # download again and verify checksums
./scripts/fetch-assets.sh --check  # only verify what is already present
```

`scripts/checksums.sha256` pins the exact bytes that were downloaded, so a
changed upstream file is noticed. If an upstream document is updated, edit the
URL/checksums deliberately rather than blindly overwriting.
