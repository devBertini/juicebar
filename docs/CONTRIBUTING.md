# Contributing to JuiceBar

JuiceBar shows the battery of wireless devices next to the clock, on macOS and Windows. The most useful contribution is support for a device or brand it does not read yet.

## Adding a device or a brand
1. Read `spec/PROVIDERS.md`: what a provider reports, the rules it follows (event-driven, read only, coexisting with vendor software, no secrets, no network) and the providers that exist.
2. Capture your device with the probe of your platform (`windows/probe` on Windows; the Mac probe in `scripts/`): its identity, how it reports battery, and a short trace while it connects, disconnects and changes level. Remove serial numbers and addresses before you share a trace; the probes never request them.
3. Write the provider for one platform or both, with decoder tests that read your trace from `spec/fixtures/`.
4. Open a pull request with the trace, the tests and the sources of every protocol fact (vendor documentation, or the project and file where it is described, with its address).

A provider is merged only when it was proven on a real device, by you or by the maintainer.

## Rules
- **No copied code.** Protocol facts (which bytes ask for the battery, what the answer means) may come from any public source, cited. Code, and data tables such as voltage curves, may not be copied from projects under another license (GPL projects included): write your own, and calibrate curves from your own device's measurements.
- **Lightweight by design.** No polling loops, no background threads that wake while nothing changes, no new dependency without a discussion first (the Windows app depends only on Microsoft's `windows` crate; the Mac app on no package).
- **Every source file** starts with `Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.` for files of the project; a contributed file adds its author's line below it. Contributions are under the MIT License (`LICENSE`).
- English in code, comments and documents; the apps ship in English and Brazilian Portuguese.
