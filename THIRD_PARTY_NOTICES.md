# Third-party notices

## Binary dependency and asset notices

`assets/licenses/COMPONENTS.json` records the resolved Cargo dependency versions,
license expressions and source URLs. `assets/licenses/DEPENDENCY_LICENSES.txt`
bundles their available license/notice texts, including the embedded font notice.
It conservatively includes build, test and alternate-platform dependencies.
These third-party licenses remain independent of Nera's proprietary code license.
The embedded Fira Mono font has its separate SIL Open Font License in
`assets/licenses/FiraMono-LICENSE.txt`, obtained from Bevy's matching 0.19.1
source release (`assets/fonts/FiraMono-LICENSE`).

Default avatar: `siroihakumai`, model `m2`, VRM 1.0 license metadata permitting
redistribution and corporate commercial use; license URL
https://vrm.dev/licenses/1.0/. The owner confirmed distribution authority for
supplied avatar, Cloud/workstation models and animations on 2026-10-13.
This confirmation does not grant rights to arbitrary user-imported content.

Food props: Kenney Food Kit (CC0), Study book: Quaternius via Poly Pizza (CC0).
Alarm and pet audio sources, modifications and licenses are recorded alongside
the bundled assets in `assets/sound/CREDITS.md` and `assets/sound/pet/CREDITS.md`.
Classic is now derived from the documented CC0 Clock Beeps; the prior unverified
MP3 is excluded. The supplied branding is retained without an artwork redesign.
Consult the individual `CREDITS.md` and `LICENSE-*.txt` files in each asset folder.

## VRM 0.x migration mapping

Portions of the VRM 0.x to VRM 1.0 migration mapping in
`src/vrm0_migration.rs` are based on
[ZaberKo/vrm-studio](https://github.com/ZaberKo/vrm-studio), used under the MIT
License.

Copyright (c) 2026 ZaberKo

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
