# Credits and third-party licences

The controller (`gmc300e.sh`), the OpenCode skill files under `skill/`, and the
original documentation in this repository are licensed under the MIT License
(see `LICENSE`).

The material below is **third-party** and is **not** covered by this
repository's MIT License; all rights remain with their respective owners.

- The GQ manuals and protocol are copyrighted by GQ Electronics LLC. They are
  **deliberately not committed** to this repository (see `.gitignore`).
  `scripts/fetch-assets.sh` downloads them from the manufacturer's site when
  you want local copies. "Free download" is not the same as "free to
  redistribute".
- The photographs are Creative Commons licensed and **are committed**, with the
  attribution and licence terms reproduced below. Keep those notices if you
  redistribute them.

## Manuals and protocol — GQ Electronics LLC

| Local file (downloaded, **not committed**) | Description | Upstream source |
| --- | --- | --- |
| `docs/manual/GMC-300E-Plus-User-Guide.pdf` | *GQ GMC-300E Plus / GMC-300S Geiger Counter User Guide*, Revision 2.13, June 2022 (24 pages) | <https://www.gqelectronicsllc.com/GMC_300E_Plus_User_Guide.pdf> |
| `docs/manual/GMC-Quick-Start-Guide.pdf` | *GQ GMC Series Geiger Counter Quick Start Guide*, V1.9 (6 pages) | <https://www.gqelectronicsllc.com/download/GMCQuickStartGuide.pdf> |
| `docs/protocol/GQ-RFC1201.txt` | *GQ Geiger Counter Communication Protocol*, GQ-RFC1201, Ver 1.40, Jan 2015 | <https://gqelectronicsllc.com/download/GQ-RFC1201.txt> |

Copyright © GQ Electronics LLC. The manual and quick-start guide are published
by the manufacturer as free downloads. The RFC1201 text carries the notice
"Distribution of this memo is unlimited" as well as a GQ Electronics copyright
notice. `scripts/fetch-assets.sh` fetches them unchanged; their expected bytes
are pinned in `scripts/checksums.sha256`.

The tested unit reports itself as `GMC-300Re 4.70`, which belongs to the same
GMC-300E Plus / 300S family as the user guide. Some menu items and electrical
details may differ across firmware versions — always trust `GETVER` and the
actual device over the paper manual.

## Photographs

### `assets/gmc-300e-in-use.jpg`

- Title: *Zaporizhzhia geigercounter IMG 7309 (53022159432).jpg*
- Author: **Enno Lenze** (<https://www.flickr.com/people/196583442@N02>)
- Source: <https://commons.wikimedia.org/wiki/File:Zaporizhzhia_geigercounter_IMG_7309_(53022159432).jpg>
  (original: <https://www.flickr.com/photos/enno-lenze/53022159432/>)
- Licence: [Creative Commons Attribution 2.0 Generic (CC BY 2.0)](https://creativecommons.org/licenses/by/2.0/)
- Changes: none. 4032 × 3024 JPEG, exactly as published on Wikimedia Commons.
- Shown in the README as an example of a GMC-300E in the field.

### `assets/gmc-300s-white.png`

- Title: *GQ GMC-300S on a White Background.tif*
- Author: **Aerra Carnicom** (<https://commons.wikimedia.org/wiki/User:Aerrapc>)
- Source: <https://commons.wikimedia.org/wiki/File:GQ_GMC-300S_on_a_White_Background.tif>
- Licence: [Creative Commons Attribution-ShareAlike 4.0 International (CC BY-SA 4.0)](https://creativecommons.org/licenses/by-sa/4.0/)
- Changes: **downscaled** from the 3834 × 2556 TIFF original to a 1280 × 853
  PNG for use in a README. The photograph is focus-stacked; see the Commons
  file page for details.
- This is the sibling **GMC-300S**, not the GMC-300E/300Re tested here. It is
  included as a clean product-style image of the same 300 series.

CC BY-SA requires that adaptations be shared under the same or a compatible
licence; that obligation applies to this image file, not to the rest of the
repository.
